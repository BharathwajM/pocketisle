//! Decode album art / app logos with WIC into small premultiplied BGRA buffers.

use windows::core::Result;
use windows::Storage::Streams::{DataReader, IRandomAccessStreamReference};
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};

use crate::events::Art;

pub fn read_stream(r: &IRandomAccessStreamReference) -> Result<Vec<u8>> {
    let stream = r.OpenReadAsync()?.join()?;
    let size = stream.Size()? as u32;
    let reader = DataReader::CreateDataReader(&stream)?;
    reader.LoadAsync(size)?.join()?;
    let mut buf = vec![0u8; size as usize];
    reader.ReadBytes(&mut buf)?;
    Ok(buf)
}

pub fn decode(bytes: &[u8], max: u32) -> Result<Art> {
    unsafe {
        let f: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let stream = f.CreateStream()?;
        stream.InitializeFromMemory(bytes)?;
        let dec = f.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand)?;
        let frame = dec.GetFrame(0)?;
        let (mut w, mut h) = (0u32, 0u32);
        frame.GetSize(&mut w, &mut h)?;
        let s = (max as f32 / w.max(h).max(1) as f32).min(1.0);
        let (tw, th) = (((w as f32 * s).round() as u32).max(1), ((h as f32 * s).round() as u32).max(1));
        let scaler = f.CreateBitmapScaler()?;
        scaler.Initialize(&frame, tw, th, WICBitmapInterpolationModeHighQualityCubic)?;
        let conv = f.CreateFormatConverter()?;
        conv.Initialize(
            &scaler,
            &GUID_WICPixelFormat32bppPBGRA,
            WICBitmapDitherTypeNone,
            None,
            0.0,
            WICBitmapPaletteTypeMedianCut,
        )?;
        let stride = tw * 4;
        let mut px = vec![0u8; (stride * th) as usize];
        conv.CopyPixels(std::ptr::null(), stride, &mut px)?;
        let tint = vivid(average(&px));
        Ok(Art { w: tw, h: th, px, tint })
    }
}

fn average(px: &[u8]) -> [f32; 3] {
    let (mut r, mut g, mut b, mut n) = (0f64, 0f64, 0f64, 0f64);
    for p in px.chunks_exact(4) {
        let a = p[3] as f64 / 255.0;
        if a < 0.5 {
            continue;
        }
        // Weight saturated pixels more, so a grey border doesn't wash out the colour.
        let (pb, pg, pr) = (p[0] as f64, p[1] as f64, p[2] as f64);
        let sat = (pr.max(pg).max(pb) - pr.min(pg).min(pb)) / 255.0;
        let wgt = 0.2 + sat * sat * 4.0;
        r += pr * wgt;
        g += pg * wgt;
        b += pb * wgt;
        n += wgt * 255.0;
    }
    if n == 0.0 {
        return [0.6, 0.6, 0.6];
    }
    [(r / n) as f32, (g / n) as f32, (b / n) as f32]
}

/// Push a colour into a range that reads well as an accent on a dark pill.
fn vivid(c: [f32; 3]) -> [f32; 3] {
    let (h, s, l) = rgb_to_hsl(c);
    let s = if s < 0.08 { s } else { s.max(0.5) };
    hsl_to_rgb(h, s, l.clamp(0.58, 0.72))
}

pub fn rgb_to_hsl([r, g, b]: [f32; 3]) -> (f32, f32, f32) {
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.0;
    if max == min {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h / 6.0, s, l)
}

pub fn hsl_to_rgb(h: f32, s: f32, l: f32) -> [f32; 3] {
    if s == 0.0 {
        return [l, l, l];
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let f = |mut t: f32| {
        if t < 0.0 { t += 1.0; }
        if t > 1.0 { t -= 1.0; }
        if t < 1.0 / 6.0 { p + (q - p) * 6.0 * t }
        else if t < 0.5 { q }
        else if t < 2.0 / 3.0 { p + (q - p) * (2.0 / 3.0 - t) * 6.0 }
        else { p }
    };
    [f(h + 1.0 / 3.0), f(h), f(h - 1.0 / 3.0)]
}
