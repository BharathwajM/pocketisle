//! GPU drawing: Direct3D 11 device -> DXGI composition swap chain (premultiplied alpha) ->
//! DirectComposition visual on the window, drawn with Direct2D + DirectWrite.
//! All drawing coordinates are logical pixels (DIPs); the device context's DPI handles scaling.

use std::cell::RefCell;
use std::collections::HashMap;
use std::mem::ManuallyDrop;

use windows::core::{Interface, Result, HSTRING};
use windows::Win32::Foundation::{HMODULE, HWND, RECT};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::DirectComposition::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows_numerics::{Matrix3x2, Vector2};

use crate::events::Art;
use crate::theme::Rgba;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct R {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl R {
    pub fn new(x0: f32, y0: f32, x1: f32, y1: f32) -> Self {
        Self { x0, y0, x1, y1 }
    }
    pub fn xywh(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x0: x, y0: y, x1: x + w, y1: y + h }
    }
    pub fn w(&self) -> f32 {
        self.x1 - self.x0
    }
    pub fn h(&self) -> f32 {
        self.y1 - self.y0
    }
    pub fn cy(&self) -> f32 {
        (self.y0 + self.y1) / 2.0
    }
    pub fn cx(&self) -> f32 {
        (self.x0 + self.x1) / 2.0
    }
    pub fn offset(&self, dx: f32, dy: f32) -> Self {
        Self { x0: self.x0 + dx, y0: self.y0 + dy, x1: self.x1 + dx, y1: self.y1 + dy }
    }
    pub fn inset(&self, d: f32) -> Self {
        Self { x0: self.x0 + d, y0: self.y0 + d, x1: self.x1 - d, y1: self.y1 - d }
    }
    pub fn union(&self, o: &R) -> Self {
        Self { x0: self.x0.min(o.x0), y0: self.y0.min(o.y0), x1: self.x1.max(o.x1), y1: self.y1.max(o.y1) }
    }
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x0 && x <= self.x1 && y >= self.y0 && y <= self.y1
    }
    fn d2d(&self) -> D2D_RECT_F {
        D2D_RECT_F { left: self.x0, top: self.y0, right: self.x1, bottom: self.y1 }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// Radii that fit the island: bottom corners no bigger than half the width or the height
/// below the shoulders.
fn island_radii(body: R, rad: f32, s: f32) -> (f32, f32) {
    let s = s.clamp(0.0, (body.h() / 2.0).max(0.0));
    let rad = rad.clamp(0.0, (body.w() / 2.0).min(body.h() - s).max(0.0));
    (rad, s)
}

/// The island's outline as a polygon (for the window's hit-test region).
pub fn island_outline(body: R, rad: f32, s: f32) -> Vec<(f32, f32)> {
    use std::f32::consts::{FRAC_PI_2, PI};
    let (rad, s) = island_radii(body, rad, s);
    let mut pts = Vec::with_capacity(40);
    let mut arc = |cx: f32, cy: f32, r: f32, a0: f32, a1: f32| {
        for i in 0..=8 {
            let a = a0 + (a1 - a0) * i as f32 / 8.0;
            pts.push((cx + r * a.cos(), cy + r * a.sin()));
        }
    };
    // Left shoulder (centre outside the island), bottom corners, right shoulder.
    arc(body.x0 - s, body.y0 + s, s, -FRAC_PI_2, 0.0);
    arc(body.x0 + rad, body.y1 - rad, rad, PI, FRAC_PI_2);
    arc(body.x1 - rad, body.y1 - rad, rad, FRAC_PI_2, 0.0);
    arc(body.x1 + s, body.y0 + s, s, PI, PI + FRAC_PI_2);
    pts
}

fn color(c: Rgba) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r: c.r, g: c.g, b: c.b, a: c.a }
}

pub struct Renderer {
    factory: ID2D1Factory1,
    dc: ID2D1DeviceContext,
    swap: IDXGISwapChain1,
    _dcomp: IDCompositionDevice,
    _target: IDCompositionTarget,
    _visual: IDCompositionVisual,
    /// The graphics device, kept so islands on other screens can draw with it too (and share
    /// its pictures: album art, sprites).
    d3d: ID3D11Device,
    d2d_device: ID2D1Device,
    /// Soft-glow pictures by colour, made once and reused every frame.
    glows: RefCell<HashMap<u32, ID2D1Bitmap1>>,
    /// Present's sync interval: the main island waits for the screen's refresh, the others
    /// don't (so two islands never take two refreshes per frame).
    sync: u32,
    dwrite: IDWriteFactory,
    brush: ID2D1SolidColorBrush,
    formats: HashMap<(bool, u32, u32, bool), IDWriteTextFormat>,
    round_caps: Option<ID2D1StrokeStyle>,
    pub font: String,
    pub icon_font: String,
    scale: f32,
}

impl Renderer {
    /// `gpu`: draw on the graphics card. Off by default: the pill is small enough that
    /// Direct3D's software rasteriser (WARP) uses less CPU *and* avoids loading the GPU
    /// driver (measured: 18 MB vs 52 MB private memory, 0.6% vs 1.0% CPU while animating).
    pub fn new(hwnd: HWND, w: u32, h: u32, scale: f32, font: &str, icon_font: &str, gpu: bool) -> Result<Self> {
        unsafe {
            let mut device = None;
            let flags = D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_SINGLETHREADED;
            let first = if gpu { D3D_DRIVER_TYPE_HARDWARE } else { D3D_DRIVER_TYPE_WARP };
            if D3D11CreateDevice(None, first, HMODULE::default(), flags, None, D3D11_SDK_VERSION, Some(&mut device), None, None).is_err() {
                D3D11CreateDevice(None, D3D_DRIVER_TYPE_WARP, HMODULE::default(), flags, None, D3D11_SDK_VERSION, Some(&mut device), None, None)?;
            }
            let device: ID3D11Device = device.unwrap();
            let dxgi: IDXGIDevice = device.cast()?;
            // Queue at most one frame, so Present(1) really waits for the next vblank.
            if let Ok(d1) = device.cast::<IDXGIDevice1>() {
                let _ = d1.SetMaximumFrameLatency(1);
            }

            let factory: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, Some(&D2D1_FACTORY_OPTIONS::default()))?;
            let d2d_device = factory.CreateDevice(&dxgi)?;
            Self::build(device, factory, d2d_device, hwnd, w, h, scale, font, icon_font, 1)
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn build(
        device: ID3D11Device,
        factory: ID2D1Factory1,
        d2d_device: ID2D1Device,
        hwnd: HWND,
        w: u32,
        h: u32,
        scale: f32,
        font: &str,
        icon_font: &str,
        sync: u32,
    ) -> Result<Self> {
        unsafe {
            let dxgi: IDXGIDevice = device.cast()?;
            let dc = d2d_device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;

            let dxgi_factory: IDXGIFactory2 = CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0))?;
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: w,
                Height: h,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
                ..Default::default()
            };
            let swap = dxgi_factory.CreateSwapChainForComposition(&device, &desc, None)?;

            let dcomp: IDCompositionDevice = DCompositionCreateDevice(&dxgi)?;
            let target = dcomp.CreateTargetForHwnd(hwnd, true)?;
            let visual = dcomp.CreateVisual()?;
            visual.SetContent(&swap)?;
            target.SetRoot(&visual)?;
            dcomp.Commit()?;

            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let brush = dc.CreateSolidColorBrush(&color(Rgba::rgb(255, 255, 255)), None)?;

            let r = Self {
                factory,
                dc,
                swap,
                _dcomp: dcomp,
                _target: target,
                _visual: visual,
                d3d: device,
                d2d_device,
                glows: RefCell::new(HashMap::new()),
                sync,
                dwrite,
                brush,
                formats: HashMap::new(),
                round_caps: None,
                font: font.to_string(),
                icon_font: icon_font.to_string(),
                scale,
            };
            r.bind()?;
            Ok(r)
        }
    }

    fn bind(&self) -> Result<()> {
        unsafe {
            let surface: IDXGISurface = self.swap.GetBuffer(0)?;
            let dpi = 96.0 * self.scale;
            let props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                dpiX: dpi,
                dpiY: dpi,
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                colorContext: ManuallyDrop::new(None),
            };
            let bmp = self.dc.CreateBitmapFromDxgiSurface(&surface, Some(&props))?;
            self.dc.SetTarget(&bmp);
            self.dc.SetDpi(dpi, dpi);
            self.dc.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        }
        Ok(())
    }

    pub fn resize(&mut self, w: u32, h: u32, scale: f32) -> Result<()> {
        unsafe {
            self.dc.SetTarget(None::<&ID2D1Image>);
            self.swap.ResizeBuffers(2, w, h, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SWAP_CHAIN_FLAG(0))?;
        }
        self.scale = scale;
        self.bind()
    }

    /// A renderer for another window (the island on another screen) on the same device, so
    /// bitmaps made by either can be drawn by both.
    pub fn sibling(&self, hwnd: HWND, w: u32, h: u32, scale: f32) -> Result<Self> {
        Self::build(self.d3d.clone(), self.factory.clone(), self.d2d_device.clone(), hwnd, w, h, scale, &self.font, &self.icon_font, 0)
    }

    pub fn set_fonts(&mut self, font: &str, icon_font: &str) {
        self.font = font.to_string();
        self.icon_font = icon_font.to_string();
        self.formats.clear();
    }

    /// Start a frame that only touches `dirty` (logical px). Everything outside it must be
    /// unchanged in both swap-chain buffers, which the caller guarantees by passing the union
    /// of this frame's and the previous two frames' drawn areas.
    pub fn begin(&self, dirty: R) {
        unsafe {
            self.dc.BeginDraw();
            self.dc.SetTransform(&Matrix3x2::identity());
            self.dc.PushAxisAlignedClip(&dirty.d2d(), D2D1_ANTIALIAS_MODE_ALIASED);
            self.dc.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));
        }
    }

    pub fn end(&self, dirty: R) -> Result<()> {
        unsafe {
            self.dc.PopAxisAlignedClip();
            self.dc.EndDraw(None, None)?;
            let desc = self.swap.GetDesc1()?;
            let s = self.scale;
            let mut rect = RECT {
                left: ((dirty.x0 * s).floor() as i32).clamp(0, desc.Width as i32),
                top: ((dirty.y0 * s).floor() as i32).clamp(0, desc.Height as i32),
                right: ((dirty.x1 * s).ceil() as i32).clamp(0, desc.Width as i32),
                bottom: ((dirty.y1 * s).ceil() as i32).clamp(0, desc.Height as i32),
            };
            if rect.right <= rect.left || rect.bottom <= rect.top {
                rect = RECT { left: 0, top: 0, right: 1, bottom: 1 };
            }
            let params = DXGI_PRESENT_PARAMETERS {
                DirtyRectsCount: 1,
                pDirtyRects: &mut rect,
                pScrollRect: std::ptr::null_mut(),
                pScrollOffset: std::ptr::null_mut(),
            };
            self.swap.Present1(self.sync, DXGI_PRESENT(0), &params).ok()
        }
    }

    fn paint(&self, c: Rgba) -> &ID2D1SolidColorBrush {
        unsafe { self.brush.SetColor(&color(c)) };
        &self.brush
    }

    pub fn fill_round(&self, r: R, rad: f32, c: Rgba) {
        let rr = D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: rad, radiusY: rad };
        unsafe { self.dc.FillRoundedRectangle(&rr, self.paint(c)) };
    }

    pub fn stroke_round(&self, r: R, rad: f32, c: Rgba, width: f32) {
        let rr = D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: rad, radiusY: rad };
        unsafe { self.dc.DrawRoundedRectangle(&rr, self.paint(c), width, None) };
    }

    pub fn circle(&self, cx: f32, cy: f32, rad: f32, c: Rgba) {
        let e = D2D1_ELLIPSE { point: Vector2 { X: cx, Y: cy }, radiusX: rad, radiusY: rad };
        unsafe { self.dc.FillEllipse(&e, self.paint(c)) };
    }

    /// A ring outline.
    pub fn ring(&self, cx: f32, cy: f32, rad: f32, c: Rgba, w: f32) {
        let e = D2D1_ELLIPSE { point: Vector2 { X: cx, Y: cy }, radiusX: rad, radiusY: rad };
        unsafe { self.dc.DrawEllipse(&e, self.paint(c), w, None) };
    }

    /// A round-capped arc starting at 12 o'clock, clockwise, covering `frac` of the circle.
    pub fn arc(&mut self, cx: f32, cy: f32, rad: f32, frac: f32, c: Rgba, w: f32) {
        let frac = frac.clamp(0.0, 1.0);
        if frac <= 0.0 {
            return;
        }
        if frac >= 0.999 {
            return self.ring(cx, cy, rad, c, w);
        }
        unsafe {
            if self.round_caps.is_none() {
                let props = D2D1_STROKE_STYLE_PROPERTIES {
                    startCap: D2D1_CAP_STYLE_ROUND,
                    endCap: D2D1_CAP_STYLE_ROUND,
                    dashCap: D2D1_CAP_STYLE_ROUND,
                    lineJoin: D2D1_LINE_JOIN_ROUND,
                    miterLimit: 1.0,
                    dashStyle: D2D1_DASH_STYLE_SOLID,
                    dashOffset: 0.0,
                };
                self.round_caps = ID2D1Factory::CreateStrokeStyle(&*self.factory, &props, None).ok();
            }
            let Ok(geom) = self.factory.CreatePathGeometry() else { return };
            let Ok(sink) = geom.Open() else { return };
            let a0 = -std::f32::consts::FRAC_PI_2;
            let a1 = a0 + frac * std::f32::consts::TAU;
            sink.BeginFigure(Vector2 { X: cx + rad * a0.cos(), Y: cy + rad * a0.sin() }, D2D1_FIGURE_BEGIN_HOLLOW);
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: Vector2 { X: cx + rad * a1.cos(), Y: cy + rad * a1.sin() },
                size: D2D_SIZE_F { width: rad, height: rad },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                arcSize: if frac > 0.5 { D2D1_ARC_SIZE_LARGE } else { D2D1_ARC_SIZE_SMALL },
            });
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
            if sink.Close().is_err() {
                return;
            }
            self.dc.DrawGeometry(&geom, self.paint(c), w, self.round_caps.as_ref());
        }
    }

    /// A filled shape through these points (closed back to the first).
    pub fn poly(&mut self, pts: &[(f32, f32)], c: Rgba) {
        if pts.len() < 3 || c.a <= 0.0 {
            return;
        }
        let v: Vec<Vector2> = pts.iter().map(|(x, y)| Vector2 { X: *x, Y: *y }).collect();
        unsafe {
            let Ok(g) = self.factory.CreatePathGeometry() else { return };
            let Ok(sink) = g.Open() else { return };
            sink.BeginFigure(v[0], D2D1_FIGURE_BEGIN_FILLED);
            sink.AddLines(&v[1..]);
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            if sink.Close().is_err() {
                return;
            }
            self.dc.FillGeometry(&g, self.paint(c), None);
        }
    }

    /// A filled line graph of `vals` (0..1), newest on the right, `cap` samples wide.
    pub fn sparkline(&mut self, r: R, vals: &[f32], cap: usize, c: Rgba) {
        if vals.len() < 2 || cap < 2 {
            return;
        }
        let step = r.w() / (cap - 1) as f32;
        let n = vals.len().min(cap);
        let pts: Vec<Vector2> = vals[vals.len() - n..]
            .iter()
            .enumerate()
            .map(|(i, v)| Vector2 { X: r.x1 - (n - 1 - i) as f32 * step, Y: r.y1 - v.clamp(0.0, 1.0) * r.h() })
            .collect();
        unsafe {
            let build = |closed: bool| -> Option<ID2D1PathGeometry1> {
                let g = self.factory.CreatePathGeometry().ok()?;
                let sink = g.Open().ok()?;
                if closed {
                    sink.BeginFigure(Vector2 { X: pts[0].X, Y: r.y1 }, D2D1_FIGURE_BEGIN_FILLED);
                    sink.AddLines(&pts);
                    sink.AddLine(Vector2 { X: pts[n - 1].X, Y: r.y1 });
                    sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                } else {
                    sink.BeginFigure(pts[0], D2D1_FIGURE_BEGIN_HOLLOW);
                    sink.AddLines(&pts[1..]);
                    sink.EndFigure(D2D1_FIGURE_END_OPEN);
                }
                sink.Close().ok()?;
                Some(g)
            };
            if let Some(g) = build(true) {
                self.dc.FillGeometry(&g, self.paint(c.a(0.18)), None);
            }
            if let Some(g) = build(false) {
                self.dc.DrawGeometry(&g, self.paint(c), 1.5, None);
            }
        }
    }

    fn format(&mut self, icon: bool, size: f32, weight: u32, wrap: bool) -> Option<IDWriteTextFormat> {
        let key = (icon, (size * 10.0) as u32, weight, wrap);
        if let Some(f) = self.formats.get(&key) {
            return Some(f.clone());
        }
        let family = HSTRING::from(if icon { self.icon_font.as_str() } else { self.font.as_str() });
        unsafe {
            let f = self
                .dwrite
                .CreateTextFormat(&family, None, DWRITE_FONT_WEIGHT(weight as i32), DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, size, &HSTRING::from("en-us"))
                .ok()?;
            if wrap {
                let _ = f.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP);
                let _ = f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR);
            } else {
                let _ = f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
                let _ = f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
            }
            if let Ok(sign) = self.dwrite.CreateEllipsisTrimmingSign(&f) {
                let trim = DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiterCount: 0 };
                let _ = f.SetTrimming(&trim, &sign);
            }
            self.formats.insert(key, f.clone());
            Some(f)
        }
    }

    pub fn text(&mut self, s: &str, r: R, size: f32, weight: u32, c: Rgba, align: Align) {
        self.draw_text(s, r, size, weight, c, align, false, false)
    }

    /// Text wrapped over as many lines as fit in `r`, with an ellipsis if it runs out of room.
    pub fn text_wrapped(&mut self, s: &str, r: R, size: f32, weight: u32, c: Rgba) {
        self.draw_text(s, r, size, weight, c, Align::Left, false, true)
    }

    pub fn icon(&mut self, s: &str, r: R, size: f32, c: Rgba) {
        self.draw_text(s, r, size, 400, c, Align::Center, true, false)
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_text(&mut self, s: &str, r: R, size: f32, weight: u32, c: Rgba, align: Align, icon: bool, wrap: bool) {
        if s.is_empty() || r.w() <= 1.0 {
            return;
        }
        let Some(f) = self.format(icon, size, weight, wrap) else { return };
        let wide: Vec<u16> = s.encode_utf16().collect();
        unsafe {
            let _ = f.SetTextAlignment(match align {
                Align::Left => DWRITE_TEXT_ALIGNMENT_LEADING,
                Align::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
                Align::Right => DWRITE_TEXT_ALIGNMENT_TRAILING,
            });
            self.dc.DrawText(
                &wide,
                &f,
                &r.d2d(),
                self.paint(c),
                D2D1_DRAW_TEXT_OPTIONS_CLIP | D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }

    pub fn measure(&mut self, s: &str, size: f32, weight: u32) -> f32 {
        let Some(f) = self.format(false, size, weight, false) else { return 0.0 };
        let wide: Vec<u16> = s.encode_utf16().collect();
        unsafe {
            let Ok(layout) = self.dwrite.CreateTextLayout(&wide, &f, 2000.0, 100.0) else { return 0.0 };
            let mut m = DWRITE_TEXT_METRICS::default();
            if layout.GetMetrics(&mut m).is_ok() {
                m.widthIncludingTrailingWhitespace.ceil()
            } else {
                0.0
            }
        }
    }

    pub fn wrapped_height(&mut self, s: &str, width: f32, size: f32, weight: u32) -> f32 {
        if s.is_empty() { return 0.0; }
        let Some(format) = self.format(false, size, weight, true) else { return 0.0 };
        let wide: Vec<u16> = s.encode_utf16().collect();
        unsafe {
            let Ok(layout) = self.dwrite.CreateTextLayout(&wide, &format, width, 10000.0) else { return 0.0 };
            let mut metrics = DWRITE_TEXT_METRICS::default();
            if layout.GetMetrics(&mut metrics).is_ok() { metrics.height.ceil() } else { 0.0 }
        }
    }

    pub fn bitmap(&self, art: &Art) -> Option<ID2D1Bitmap1> {
        let props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
            colorContext: ManuallyDrop::new(None),
        };
        unsafe {
            self.dc
                .CreateBitmap(D2D_SIZE_U { width: art.w, height: art.h }, Some(art.px.as_ptr() as *const _), art.w * 4, &props)
                .ok()
        }
    }

    /// Part of a bitmap (`src`, in its pixels) into `dest`, optionally mirrored left-to-right
    /// about its own centre: crisp (nearest neighbour) for pixel art drawn at whole-pixel
    /// scale, smooth when shrinking it.
    #[allow(clippy::too_many_arguments)]
    pub fn sprite_flip(&self, bmp: &ID2D1Bitmap1, dest: R, src: R, crisp: bool, flip: bool, opacity: f32) {
        unsafe {
            let mode = if crisp { D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR } else { D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC };
            if !flip {
                self.dc.DrawBitmap(bmp, Some(&dest.d2d()), opacity, mode, Some(&src.d2d()), None);
                return;
            }
            let mut m = Matrix3x2 { M11: 1.0, M12: 0.0, M21: 0.0, M22: 1.0, M31: 0.0, M32: 0.0 };
            self.dc.GetTransform(&mut m);
            // Mirror x about the centre first (x -> x0 + x1 - x), then the current transform.
            let c = dest.x0 + dest.x1;
            let f = Matrix3x2 { M11: -m.M11, M12: -m.M12, M21: m.M21, M22: m.M22, M31: c * m.M11 + m.M31, M32: c * m.M12 + m.M32 };
            self.dc.SetTransform(&f);
            self.dc.DrawBitmap(bmp, Some(&dest.d2d()), opacity, mode, Some(&src.d2d()), None);
            self.dc.SetTransform(&m);
        }
    }

    /// Draw an image cropped to fill `r` (like CSS object-fit: cover), with rounded corners.
    pub fn image(&self, bmp: &ID2D1Bitmap1, r: R, rad: f32, opacity: f32) {
        unsafe {
            let size = bmp.GetSize();
            let s = (r.w() / size.width).max(r.h() / size.height);
            let (dw, dh) = (size.width * s, size.height * s);
            let (ox, oy) = (r.x0 - (dw - r.w()) / 2.0, r.y0 - (dh - r.h()) / 2.0);
            let Ok(brush) = self.dc.CreateBitmapBrush(bmp, None, None) else { return };
            brush.SetTransform(&Matrix3x2 { M11: s, M12: 0.0, M21: 0.0, M22: s, M31: ox, M32: oy });
            brush.SetOpacity(opacity);
            let rr = D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: rad, radiusY: rad };
            self.dc.FillRoundedRectangle(&rr, &brush);
        }
    }

    /// The island's outline: hangs from `body.y0` (the screen's top edge) with concave
    /// shoulders of radius `s` flowing into the edge, straight sides and round bottom corners.
    /// With `hang` off it's a plain rounded rectangle (the floating pill).
    pub fn island(&self, body: R, rad: f32, s: f32, hang: bool) -> Option<ID2D1Geometry> {
        if !hang {
            let rr = D2D1_ROUNDED_RECT { rect: body.d2d(), radiusX: rad, radiusY: rad };
            return unsafe { self.factory.CreateRoundedRectangleGeometry(&rr).ok()?.cast().ok() };
        }
        let (rad, s) = island_radii(body, rad, s);
        let (x0, x1, y0, y1) = (body.x0, body.x1, body.y0, body.y1);
        let arc = |sink: &ID2D1GeometrySink, x: f32, y: f32, r: f32, cw: bool| unsafe {
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: Vector2 { X: x, Y: y },
                size: D2D_SIZE_F { width: r, height: r },
                rotationAngle: 0.0,
                sweepDirection: if cw { D2D1_SWEEP_DIRECTION_CLOCKWISE } else { D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE },
                arcSize: D2D1_ARC_SIZE_SMALL,
            })
        };
        unsafe {
            let g = self.factory.CreatePathGeometry().ok()?;
            let sink = g.Open().ok()?;
            sink.BeginFigure(Vector2 { X: x0 - s, Y: y0 }, D2D1_FIGURE_BEGIN_FILLED);
            if s > 0.0 {
                arc(&sink, x0, y0 + s, s, true);
            }
            sink.AddLine(Vector2 { X: x0, Y: y1 - rad });
            if rad > 0.0 {
                arc(&sink, x0 + rad, y1, rad, false);
            }
            sink.AddLine(Vector2 { X: x1 - rad, Y: y1 });
            if rad > 0.0 {
                arc(&sink, x1, y1 - rad, rad, false);
            }
            sink.AddLine(Vector2 { X: x1, Y: y0 + s });
            if s > 0.0 {
                arc(&sink, x1 + s, y0, s, true);
            }
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            sink.Close().ok()?;
            g.cast().ok()
        }
    }

    fn gradient(&self, stops: &[(f32, Rgba)]) -> Option<ID2D1GradientStopCollection> {
        let stops: Vec<D2D1_GRADIENT_STOP> = stops.iter().map(|(p, c)| D2D1_GRADIENT_STOP { position: *p, color: color(*c) }).collect();
        unsafe { ID2D1RenderTarget::CreateGradientStopCollection(&*self.dc, &stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP).ok() }
    }

    /// Fill a shape with a top-to-bottom gradient.
    fn vertical(&self, y0: f32, y1: f32, top: Rgba, bottom: Rgba) -> Option<ID2D1LinearGradientBrush> {
        let stops = self.gradient(&[(0.0, top), (1.0, bottom)])?;
        let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: Vector2 { X: 0.0, Y: y0 }, endPoint: Vector2 { X: 0.0, Y: y1 } };
        unsafe { self.dc.CreateLinearGradientBrush(&props, None, &stops).ok() }
    }

    /// Fill a shape with a top-to-bottom gradient.
    pub fn fill_vertical(&self, g: &ID2D1Geometry, y0: f32, y1: f32, top: Rgba, bottom: Rgba) {
        if let Some(b) = self.vertical(y0, y1, top, bottom) {
            unsafe { self.dc.FillGeometry(g, &b, None) };
        }
    }

    /// Outline a shape with a top-to-bottom gradient (a rim that catches the light below).
    pub fn stroke_vertical(&self, g: &ID2D1Geometry, y0: f32, y1: f32, top: Rgba, bottom: Rgba, w: f32) {
        if let Some(b) = self.vertical(y0, y1, top, bottom) {
            unsafe { self.dc.DrawGeometry(g, &b, w, None) };
        }
    }

    /// A soft round light: `c` at the centre fading to nothing at `rad`.
    pub fn glow(&self, cx: f32, cy: f32, rad: f32, c: Rgba) {
        if rad <= 0.0 || c.a <= 0.001 {
            return;
        }
        // A small soft-dot picture per colour, stretched into place and faded with its
        // opacity: far cheaper than a gradient for the software rasteriser.
        let key = ((c.r * 255.0).round() as u32) << 16 | ((c.g * 255.0).round() as u32) << 8 | (c.b * 255.0).round() as u32;
        let mut glows = self.glows.borrow_mut();
        if !glows.contains_key(&key) {
            if glows.len() > 48 {
                glows.clear();
            }
            const N: u32 = 64;
            let mut px = vec![0u8; (N * N * 4) as usize];
            for y in 0..N {
                for x in 0..N {
                    let dx = (x as f32 + 0.5) / N as f32 * 2.0 - 1.0;
                    let dy = (y as f32 + 0.5) / N as f32 * 2.0 - 1.0;
                    let d = (dx * dx + dy * dy).sqrt();
                    let a = if d < 0.55 {
                        1.0 - 0.65 * d / 0.55
                    } else if d < 1.0 {
                        0.35 * (1.0 - (d - 0.55) / 0.45)
                    } else {
                        0.0
                    };
                    let i = ((y * N + x) * 4) as usize;
                    px[i] = (c.b * a * 255.0).round() as u8;
                    px[i + 1] = (c.g * a * 255.0).round() as u8;
                    px[i + 2] = (c.r * a * 255.0).round() as u8;
                    px[i + 3] = (a * 255.0).round() as u8;
                }
            }
            let Some(bmp) = self.bitmap(&Art { w: N, h: N, px, tint: [0.0; 3] }) else { return };
            glows.insert(key, bmp);
        }
        let Some(bmp) = glows.get(&key) else { return };
        unsafe {
            let dest = R::new(cx - rad, cy - rad, cx + rad, cy + rad);
            self.dc.DrawBitmap(bmp, Some(&dest.d2d()), c.a.min(1.0), D2D1_INTERPOLATION_MODE_LINEAR, None, None);
        }
    }

    /// Many short strokes in one go (rain, rays): one shape, drawn once.
    pub fn lines(&mut self, segs: &[(f32, f32, f32, f32)], c: Rgba, w: f32) {
        if segs.is_empty() || c.a <= 0.001 {
            return;
        }
        self.ensure_round_caps();
        unsafe {
            let Ok(g) = self.factory.CreatePathGeometry() else { return };
            let Ok(sink) = g.Open() else { return };
            for &(ax, ay, bx, by) in segs {
                sink.BeginFigure(Vector2 { X: ax, Y: ay }, D2D1_FIGURE_BEGIN_HOLLOW);
                sink.AddLine(Vector2 { X: bx, Y: by });
                sink.EndFigure(D2D1_FIGURE_END_OPEN);
            }
            if sink.Close().is_err() {
                return;
            }
            self.dc.DrawGeometry(&g, self.paint(c), w, self.round_caps.as_ref());
        }
    }

    /// A travelling wave from `x0` to `x1` around `y`: `amp` high, `len` long, shifted by
    /// `phase` (radians), tapering to flat at both ends.
    #[allow(clippy::too_many_arguments)]
    pub fn wave(&mut self, x0: f32, x1: f32, y: f32, amp: f32, len: f32, phase: f32, c: Rgba, w: f32) {
        if x1 - x0 < 4.0 {
            return;
        }
        let n = ((x1 - x0) / 3.0).ceil().max(2.0) as usize;
        let pts: Vec<Vector2> = (0..=n)
            .map(|i| {
                let u = i as f32 / n as f32;
                let x = x0 + (x1 - x0) * u;
                let taper = (u * std::f32::consts::PI).sin();
                Vector2 { X: x, Y: y + amp * taper * (x / len * std::f32::consts::TAU + phase).sin() }
            })
            .collect();
        self.ensure_round_caps();
        unsafe {
            let Ok(g) = self.factory.CreatePathGeometry() else { return };
            let Ok(sink) = g.Open() else { return };
            sink.BeginFigure(pts[0], D2D1_FIGURE_BEGIN_HOLLOW);
            sink.AddLines(&pts[1..]);
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
            if sink.Close().is_err() {
                return;
            }
            self.dc.DrawGeometry(&g, self.paint(c), w, self.round_caps.as_ref());
        }
    }

    fn ensure_round_caps(&mut self) {
        if self.round_caps.is_some() {
            return;
        }
        let props = D2D1_STROKE_STYLE_PROPERTIES {
            startCap: D2D1_CAP_STYLE_ROUND,
            endCap: D2D1_CAP_STYLE_ROUND,
            dashCap: D2D1_CAP_STYLE_ROUND,
            lineJoin: D2D1_LINE_JOIN_ROUND,
            miterLimit: 1.0,
            dashStyle: D2D1_DASH_STYLE_SOLID,
            dashOffset: 0.0,
        };
        self.round_caps = unsafe { ID2D1Factory::CreateStrokeStyle(&*self.factory, &props, None).ok() };
    }

    /// A straight stroke with round ends.
    pub fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, c: Rgba, w: f32) {
        self.ensure_round_caps();
        unsafe {
            self.dc.DrawLine(Vector2 { X: x0, Y: y0 }, Vector2 { X: x1, Y: y1 }, self.paint(c), w, self.round_caps.as_ref());
        }
    }

    /// A plain filled rectangle (pixel art).
    pub fn rect(&self, r: R, c: Rgba) {
        unsafe { self.dc.FillRectangle(&r.d2d(), self.paint(c)) };
    }

    /// Clip following drawing to a shape and fade it by `opacity`.
    pub fn push_shape(&self, g: &ID2D1Geometry, opacity: f32) {
        unsafe {
            let params = D2D1_LAYER_PARAMETERS1 {
                contentBounds: D2D_RECT_F { left: -1e6, top: -1e6, right: 1e6, bottom: 1e6 },
                geometricMask: ManuallyDrop::new(Some(g.clone())),
                maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                maskTransform: Matrix3x2::identity(),
                opacity,
                opacityBrush: ManuallyDrop::new(None),
                layerOptions: D2D1_LAYER_OPTIONS1_NONE,
            };
            self.dc.PushLayer(&params, None::<&ID2D1Layer>);
            let _ = ManuallyDrop::into_inner(params.geometricMask);
        }
    }

    pub fn pop(&self) {
        unsafe { self.dc.PopLayer() };
    }
}
