//! Now playing, from Windows' system media sessions (Spotify, browsers, Media Player, …).
//! Everything is event-driven: the thread sleeps on its channel until Windows reports a change
//! or the island sends a command.

use std::sync::mpsc::{channel, Receiver, Sender};

use windows::core::Result;
use windows::Foundation::TypedEventHandler;
use windows::Media::Control::*;
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

use crate::events::{self, Event, MediaInfo};

pub enum MediaCmd {
    Session,
    Props,
    Playback,
    Timeline,
    PlayPause,
    Next,
    Prev,
    /// Jump to this many seconds into the track.
    Seek(f64),
}

pub fn spawn() -> Sender<MediaCmd> {
    let (tx, rx) = channel();
    let tx2 = tx.clone();
    std::thread::spawn(move || {
        if let Err(e) = run(rx, tx2) {
            crate::util::log(&format!("media watcher stopped: {e}"));
        }
    });
    tx
}

type Session = GlobalSystemMediaTransportControlsSession;

struct Current {
    session: Session,
    tokens: [i64; 3],
}

fn run(rx: Receiver<MediaCmd>, tx: Sender<MediaCmd>) -> Result<()> {
    unsafe {
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
    }
    let mgr = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()?.join()?;
    {
        let tx = tx.clone();
        mgr.CurrentSessionChanged(&TypedEventHandler::new(move |_, _| {
            let _ = tx.send(MediaCmd::Session);
            Ok(())
        }))?;
    }
    {
        let tx = tx.clone();
        mgr.SessionsChanged(&TypedEventHandler::new(move |_, _| {
            let _ = tx.send(MediaCmd::Session);
            Ok(())
        }))?;
    }
    let _ = tx.send(MediaCmd::Session);

    let mut cur: Option<Current> = None;
    let mut last_title = String::new();
    let mut have_art = false;

    for cmd in rx {
        match cmd {
            MediaCmd::Session => {
                if let Some(c) = cur.take() {
                    let _ = c.session.RemoveMediaPropertiesChanged(c.tokens[0]);
                    let _ = c.session.RemovePlaybackInfoChanged(c.tokens[1]);
                    let _ = c.session.RemoveTimelinePropertiesChanged(c.tokens[2]);
                }
                match mgr.GetCurrentSession() {
                    Ok(s) => {
                        let t0 = {
                            let tx = tx.clone();
                            s.MediaPropertiesChanged(&TypedEventHandler::new(move |_, _| {
                                let _ = tx.send(MediaCmd::Props);
                                Ok(())
                            }))?
                        };
                        let t1 = {
                            let tx = tx.clone();
                            s.PlaybackInfoChanged(&TypedEventHandler::new(move |_, _| {
                                let _ = tx.send(MediaCmd::Playback);
                                Ok(())
                            }))?
                        };
                        let t2 = {
                            let tx = tx.clone();
                            s.TimelinePropertiesChanged(&TypedEventHandler::new(move |_, _| {
                                let _ = tx.send(MediaCmd::Timeline);
                                Ok(())
                            }))?
                        };
                        last_title.clear();
                        have_art = false;
                        refresh_props(&s, &mut last_title, &mut have_art);
                        refresh_playback(&s);
                        refresh_timeline(&s);
                        cur = Some(Current { session: s, tokens: [t0, t1, t2] });
                    }
                    Err(_) => {
                        last_title.clear();
                        have_art = false;
                        events::send(Event::Media(None));
                        events::send(Event::MediaArt(None));
                    }
                }
            }
            MediaCmd::Props => {
                if let Some(c) = &cur {
                    refresh_props(&c.session, &mut last_title, &mut have_art);
                }
            }
            MediaCmd::Playback => {
                if let Some(c) = &cur {
                    refresh_playback(&c.session);
                    refresh_timeline(&c.session);
                }
            }
            MediaCmd::Timeline => {
                if let Some(c) = &cur {
                    refresh_timeline(&c.session);
                }
            }
            MediaCmd::Seek(secs) => {
                if let Some(c) = &cur {
                    let start = c.session.GetTimelineProperties().and_then(|t| t.StartTime()).map(|d| d.Duration).unwrap_or(0);
                    if let Ok(op) = c.session.TryChangePlaybackPositionAsync(start + (secs.max(0.0) * 1e7) as i64) {
                        let _ = op.join();
                    }
                }
            }
            MediaCmd::PlayPause | MediaCmd::Next | MediaCmd::Prev => {
                if let Some(c) = &cur {
                    let op = match cmd {
                        MediaCmd::PlayPause => c.session.TryTogglePlayPauseAsync(),
                        MediaCmd::Next => c.session.TrySkipNextAsync(),
                        _ => c.session.TrySkipPreviousAsync(),
                    };
                    if let Ok(op) = op {
                        let _ = op.join();
                    }
                }
            }
        }
    }
    Ok(())
}

fn refresh_props(s: &Session, last_title: &mut String, have_art: &mut bool) {
    let Ok(p) = s.TryGetMediaPropertiesAsync().and_then(|op| op.join()) else { return };
    let title = p.Title().map(|h| h.to_string()).unwrap_or_default();
    let artist = p.Artist().map(|h| h.to_string()).unwrap_or_default();
    if title.is_empty() && artist.is_empty() {
        return;
    }
    let app = nice_app(&s.SourceAppUserModelId().map(|h| h.to_string()).unwrap_or_default());
    events::send(Event::Media(Some(MediaInfo { title: title.clone(), artist, app })));
    // Players often publish the title first and the artwork a moment later, so retry
    // the artwork until we have it for this track.
    if title != *last_title || !*have_art {
        *last_title = title;
        let art = p
            .Thumbnail()
            .ok()
            .and_then(|t| crate::imaging::read_stream(&t).ok())
            .and_then(|b| crate::imaging::decode(&b, 128).ok());
        *have_art = art.is_some();
        events::send(Event::MediaArt(art));
    }
}

fn refresh_playback(s: &Session) {
    let playing = s
        .GetPlaybackInfo()
        .and_then(|i| i.PlaybackStatus())
        .map(|st| st == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing)
        .unwrap_or(false);
    events::send(Event::Playback(playing));
}

fn refresh_timeline(s: &Session) {
    if let Ok(t) = s.GetTimelineProperties() {
        let ticks = |r: Result<windows::Foundation::TimeSpan>| r.map(|d| d.Duration as f64 / 1e7).unwrap_or(0.0);
        let (start, end, pos) = (ticks(t.StartTime()), ticks(t.EndTime()), ticks(t.Position()));
        events::send(Event::Timeline { pos: pos - start, end: end - start });
    }
}

fn nice_app(aumid: &str) -> String {
    let lower = aumid.to_ascii_lowercase();
    let known = [
        ("spotify", "Spotify"),
        ("chrome", "Chrome"),
        ("msedge", "Edge"),
        ("firefox", "Firefox"),
        ("308046b0af4a39cb", "Firefox"),
        ("zunemusic", "Media Player"),
        ("brave", "Brave"),
        ("opera", "Opera"),
        ("vlc", "VLC"),
        ("applemusic", "Apple Music"),
        ("youtube", "YouTube Music"),
        ("discord", "Discord"),
    ];
    if let Some((_, name)) = known.iter().find(|(k, _)| lower.contains(k)) {
        return name.to_string();
    }
    let base = aumid.rsplit(['\\', '/']).next().unwrap_or(aumid);
    let base = base.split('!').next().unwrap_or(base);
    let base = base.trim_end_matches(".exe").trim_end_matches(".EXE");
    let mut c = base.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}
