//! Pokémon cries: the OGG files downloaded next to the sprites, decoded (lewton) into an
//! in-memory WAV and played on a worker thread. One at a time; a cry asked for while another
//! is playing is skipped.

use std::sync::atomic::{AtomicBool, Ordering};

use windows::core::PCWSTR;
use windows::Win32::Media::Audio::{PlaySoundW, SND_MEMORY, SND_NODEFAULT, SND_SYNC};

static BUSY: AtomicBool = AtomicBool::new(false);

/// Play `name`'s cry (a sprite name; "-shiny" is ignored) at `volume` (0..1).
pub fn cry(name: &str, volume: f32) {
    let base = name.trim_end_matches("-shiny").to_lowercase();
    if BUSY.swap(true, Ordering::SeqCst) {
        return;
    }
    let spawned = std::thread::Builder::new().name("cry".into()).spawn(move || {
        if let Some(wav) = decode(&base, volume) {
            unsafe {
                let _ = PlaySoundW(PCWSTR(wav.as_ptr() as *const u16), None, SND_MEMORY | SND_SYNC | SND_NODEFAULT);
            }
        }
        BUSY.store(false, Ordering::SeqCst);
    });
    if spawned.is_err() {
        BUSY.store(false, Ordering::SeqCst);
    }
}

fn decode(name: &str, volume: f32) -> Option<Vec<u8>> {
    let bytes = std::fs::read(crate::util::sprite_dir().join(format!("{name}.ogg"))).ok()?;
    let mut r = lewton::inside_ogg::OggStreamReader::new(std::io::Cursor::new(bytes)).ok()?;
    let ch = r.ident_hdr.audio_channels as u16;
    let rate = r.ident_hdr.audio_sample_rate;
    let mut pcm: Vec<i16> = Vec::new();
    while let Ok(Some(p)) = r.read_dec_packet_itl() {
        pcm.extend(p.into_iter().map(|s| (s as f32 * volume) as i16));
    }
    if pcm.is_empty() || ch == 0 {
        return None;
    }
    let data = (pcm.len() * 2) as u32;
    let mut w = Vec::with_capacity(44 + data as usize);
    w.extend(b"RIFF");
    w.extend((36 + data).to_le_bytes());
    w.extend(b"WAVEfmt ");
    w.extend(16u32.to_le_bytes());
    w.extend(1u16.to_le_bytes());
    w.extend(ch.to_le_bytes());
    w.extend(rate.to_le_bytes());
    w.extend((rate * ch as u32 * 2).to_le_bytes());
    w.extend((ch * 2).to_le_bytes());
    w.extend(16u16.to_le_bytes());
    w.extend(b"data");
    w.extend(data.to_le_bytes());
    for s in pcm {
        w.extend(s.to_le_bytes());
    }
    Some(w)
}
