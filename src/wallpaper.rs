//! The island's accent colour, picked from the desktop wallpaper.
//!
//! The wallpaper is decoded small (96 px) on a worker thread and its strongest colour
//! picked from a hue histogram, then nudged into a range that reads well on the dark pill.
//! A black-and-white wallpaper gives a soft silver accent.

use std::path::PathBuf;
use std::time::SystemTime;

use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
use windows::Win32::UI::WindowsAndMessaging::{SystemParametersInfoW, SPI_GETDESKWALLPAPER, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS};

use crate::events::{self, Event};
use crate::util;

/// Identifies one wallpaper image: path, modified time and size (Windows rewrites the same
/// TranscodedWallpaper file when the picture changes).
#[derive(Clone, PartialEq, Debug)]
pub struct Key {
    pub path: PathBuf,
    pub modified: Option<SystemTime>,
    pub len: u64,
}

/// The picture currently on the desktop, if it's an image file.
pub fn current() -> Option<Key> {
    // Wallpaper Engine covers the Windows wallpaper while it runs: use its preview picture.
    if let Some(path) = crate::wpe::current_preview() {
        if let Some(m) = std::fs::metadata(&path).ok().filter(|m| m.is_file() && m.len() > 0) {
            return Some(Key { modified: m.modified().ok(), len: m.len(), path });
        }
    }
    let mut buf = [0u16; 1024];
    let set = unsafe {
        SystemParametersInfoW(SPI_GETDESKWALLPAPER, buf.len() as u32, Some(buf.as_mut_ptr() as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0)).is_ok()
    };
    let len = buf.iter().position(|c| *c == 0).unwrap_or(0);
    let mut candidates = Vec::new();
    if set && len > 0 {
        candidates.push(PathBuf::from(String::from_utf16_lossy(&buf[..len])));
    }
    // Slideshows and Spotlight leave the path empty or stale; Windows' own copy is current.
    if let Some(appdata) = std::env::var_os("APPDATA") {
        candidates.push(PathBuf::from(appdata).join(r"Microsoft\Windows\Themes\TranscodedWallpaper"));
    }
    candidates.into_iter().find_map(|path| {
        let m = std::fs::metadata(&path).ok().filter(|m| m.is_file() && m.len() > 0)?;
        Some(Key { modified: m.modified().ok(), len: m.len(), path })
    })
}

/// Work out the accent for this wallpaper in the background; sends `Event::WallpaperAccent`.
pub fn pick_async(key: Key) {
    std::thread::Builder::new()
        .name("wallpaper".into())
        .spawn(move || {
            let accent = unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                std::fs::read(&key.path)
                    .ok()
                    .and_then(|bytes| crate::imaging::decode(&bytes, 96).ok())
                    .map(|art| pick(&art.px))
            };
            if accent.is_none() {
                util::log(&format!("wallpaper: couldn't read {}", key.path.display()));
            }
            events::send(Event::WallpaperAccent(key, accent));
        })
        .ok();
}

/// The accent for a picture (premultiplied BGRA pixels).
pub fn pick(px: &[u8]) -> [f32; 3] {
    const BUCKETS: usize = 24;
    let mut weight = [0f32; BUCKETS];
    let mut sum = [[0f32; 3]; BUCKETS];
    let mut n = 0f32;
    for p in px.chunks_exact(4) {
        if p[3] < 128 {
            continue;
        }
        let c = [p[2] as f32 / 255.0, p[1] as f32 / 255.0, p[0] as f32 / 255.0];
        let (h, s, l) = crate::imaging::rgb_to_hsl(c);
        n += 1.0;
        // Colourful pixels only: not near-grey, not near-black or white.
        if s < 0.18 || !(0.1..=0.92).contains(&l) {
            continue;
        }
        // Vivid pixels count far more than muted ones, so a glow beats a brown haze.
        let chroma = s * (1.0 - (2.0 * l - 1.0).abs());
        let w = chroma * chroma;
        let b = ((h * BUCKETS as f32) as usize).min(BUCKETS - 1);
        weight[b] += w;
        for i in 0..3 {
            sum[b][i] += c[i] * w;
        }
    }
    if n == 0.0 {
        return [0.78, 0.79, 0.82];
    }
    let total: f32 = weight.iter().sum();
    // Hardly any colour (a black-and-white or grey wallpaper): soft silver.
    if total / n < 0.004 {
        return crate::imaging::hsl_to_rgb(0.6, 0.06, 0.78);
    }
    // Strongest hue, counting its neighbours so a gradient isn't split across buckets.
    let score = |i: usize| weight[(i + BUCKETS - 1) % BUCKETS] * 0.5 + weight[i] + weight[(i + 1) % BUCKETS] * 0.5;
    let best = (0..BUCKETS).max_by(|a, b| score(*a).total_cmp(&score(*b))).unwrap_or(0);
    let mut c = [0f32; 3];
    let mut w = 0f32;
    for i in [(best + BUCKETS - 1) % BUCKETS, best, (best + 1) % BUCKETS] {
        for k in 0..3 {
            c[k] += sum[i][k];
        }
        w += weight[i];
    }
    let c = [c[0] / w, c[1] / w, c[2] / w];
    let (h, s, l) = crate::imaging::rgb_to_hsl(c);
    crate::imaging::hsl_to_rgb(h, s.clamp(0.6, 0.9), l.clamp(0.6, 0.72))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(colours: &[([u8; 3], usize)]) -> Vec<u8> {
        let mut px = Vec::new();
        for (c, count) in colours {
            for _ in 0..*count {
                px.extend_from_slice(&[c[2], c[1], c[0], 255]);
            }
        }
        px
    }

    #[test]
    fn picks_strongest_colour() {
        // Mostly dark navy with a big orange sunset band.
        let px = image(&[([10, 14, 30], 600), ([240, 120, 40], 300), ([60, 140, 90], 40)]);
        let [r, g, b] = pick(&px);
        assert!(r > g && g > b, "expected orange, got {r} {g} {b}");
    }

    #[test]
    fn monochrome_gives_silver() {
        let px = image(&[([20, 20, 20], 500), ([200, 200, 200], 400), ([120, 120, 120], 100)]);
        let [r, g, b] = pick(&px);
        assert!((r - g).abs() < 0.05 && (g - b).abs() < 0.08 && r > 0.65, "expected silver, got {r} {g} {b}");
    }
}
