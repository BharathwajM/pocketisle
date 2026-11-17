//! Reads the active YASB theme (styles.css + yasb_colors.css + config.yaml) so the island
//! uses the same colours, fonts, rounding and bar geometry as the bar it sits in.

use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a: 1.0 }
    }
    pub fn from_f(c: [f32; 3]) -> Self {
        Self { r: c[0], g: c[1], b: c[2], a: 1.0 }
    }
    pub fn a(self, a: f32) -> Self {
        Self { a: self.a * a, ..self }
    }
    pub fn mix(self, o: Rgba, t: f32) -> Self {
        Self {
            r: self.r + (o.r - self.r) * t,
            g: self.g + (o.g - self.g) * t,
            b: self.b + (o.b - self.b) * t,
            a: self.a + (o.a - self.a) * t,
        }
    }

    fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if let Some(hex) = s.strip_prefix('#') {
            let v = |i: usize, n: usize| u8::from_str_radix(&hex[i..i + n], 16).ok();
            return match hex.len() {
                3 => {
                    let d = |i| v(i, 1).map(|x| x * 17);
                    Some(Self::rgb(d(0)?, d(1)?, d(2)?))
                }
                6 => Some(Self::rgb(v(0, 2)?, v(2, 2)?, v(4, 2)?)),
                8 => Some(Self::rgb(v(0, 2)?, v(2, 2)?, v(4, 2)?).a(v(6, 2)? as f32 / 255.0)),
                _ => None,
            };
        }
        let inner = s.strip_prefix("rgba(").or_else(|| s.strip_prefix("rgb("))?.strip_suffix(')')?;
        let parts: Vec<f32> = inner.split(',').filter_map(|p| p.trim().parse().ok()).collect();
        if parts.len() < 3 {
            return None;
        }
        let a = parts.get(3).copied().map(|a| if a > 1.0 { a / 255.0 } else { a }).unwrap_or(1.0);
        Some(Self { r: parts[0] / 255.0, g: parts[1] / 255.0, b: parts[2] / 255.0, a })
    }
}

#[derive(Clone)]
pub struct Theme {
    pub bg: Rgba,
    pub bg2: Rgba,
    pub accent: Rgba,
    /// The theme's own accent; kept for "watch" levels when the accent follows the wallpaper.
    pub warn: Rgba,
    pub text: Rgba,
    pub subtext: Rgba,
    pub border: Rgba,
    pub hover: Rgba,
    pub good: Rgba,
    pub bad: Rgba,
    pub font: String,
    pub icon_font: String,
    /// YASB bar padding-top and height, logical px.
    pub bar_top: f32,
    pub bar_height: f32,
}

impl Default for Theme {
    fn default() -> Self {
        // The values of the user's current YASB theme, used if the files can't be read.
        Self {
            bg: Rgba::rgb(0x13, 0x15, 0x1b),
            bg2: Rgba::rgb(0x24, 0x28, 0x32),
            accent: Rgba::rgb(0xf0, 0xc9, 0x6b),
            warn: Rgba::rgb(0xf0, 0xc9, 0x6b),
            text: Rgba::rgb(0xe6, 0xea, 0xf2),
            subtext: Rgba::rgb(0xae, 0xb8, 0xc9),
            border: Rgba::rgb(0x41, 0x49, 0x57),
            hover: Rgba::rgb(0x30, 0x3b, 0x4c),
            good: Rgba::rgb(0x8b, 0xd5, 0xa0),
            bad: Rgba::rgb(0xf3, 0x8b, 0xa8),
            font: "Segoe UI Variable Text".into(),
            icon_font: "JetBrainsMono NFP".into(),
            bar_top: 10.0,
            bar_height: 36.0,
        }
    }
}

fn strip_comments(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("/*") {
        out.push_str(&rest[..i]);
        match rest[i + 2..].find("*/") {
            Some(j) => rest = &rest[i + 2 + j + 2..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// Collect `--name: value;` declarations from every `:root { }` block; later ones win.
fn css_vars(css: &str, into: &mut HashMap<String, String>) {
    let css = strip_comments(css);
    let mut rest = css.as_str();
    while let Some(i) = rest.find(":root") {
        let Some(open) = rest[i..].find('{') else { break };
        let body_start = i + open + 1;
        let Some(close) = rest[body_start..].find('}') else { break };
        for decl in rest[body_start..body_start + close].split(';') {
            if let Some((k, v)) = decl.split_once(':') {
                let k = k.trim();
                if k.starts_with("--") {
                    into.insert(k.to_string(), v.trim().to_string());
                }
            }
        }
        rest = &rest[body_start + close..];
    }
}

fn resolve(vars: &HashMap<String, String>, name: &str) -> Option<String> {
    let mut v = vars.get(name)?.clone();
    for _ in 0..6 {
        let Some(inner) = v.trim().strip_prefix("var(").and_then(|x| x.strip_suffix(')')) else {
            break;
        };
        let key = inner.split(',').next().unwrap_or("").trim().to_string();
        v = vars.get(&key)?.clone();
    }
    Some(v)
}

fn first_font(list: &str) -> Option<String> {
    let f = list.split(',').next()?.trim().trim_matches(|c| c == '"' || c == '\'').to_string();
    if f.is_empty() {
        None
    } else if f == "Segoe UI Variable" {
        // DirectWrite exposes the variable font through its optical-size families.
        Some("Segoe UI Variable Text".into())
    } else {
        Some(f)
    }
}

/// Pull `dimensions: height` and `padding: top` of the first bar out of YASB's config.yaml.
fn bar_geometry(yaml: &str) -> (Option<f32>, Option<f32>) {
    let (mut height, mut top) = (None, None);
    let mut section = "";
    for line in yaml.lines() {
        let t = line.trim();
        if t.starts_with('#') || t.is_empty() {
            continue;
        }
        if t == "dimensions:" || t == "padding:" {
            section = if t == "dimensions:" { "dim" } else { "pad" };
            continue;
        }
        if let Some((k, v)) = t.split_once(':') {
            let val = v.trim().trim_end_matches("px").parse::<f32>().ok();
            match (section, k.trim()) {
                ("dim", "height") if height.is_none() => height = val,
                ("pad", "top") if top.is_none() => top = val,
                (_, "width" | "left" | "right" | "bottom") => {}
                _ => section = "",
            }
        }
        if height.is_some() && top.is_some() {
            break;
        }
    }
    (height, top)
}

impl Theme {
    pub fn load() -> Self {
        let mut t = Theme::default();
        let dir = crate::util::yasb_dir();
        let mut vars = HashMap::new();
        if let Ok(s) = std::fs::read_to_string(dir.join("yasb_colors.css")) {
            css_vars(&s, &mut vars);
        }
        match std::fs::read_to_string(dir.join("styles.css")) {
            Ok(s) => css_vars(&s, &mut vars),
            Err(_) => return t,
        }
        let col = |name: &str| resolve(&vars, name).and_then(|v| Rgba::parse(&v));
        if let Some(c) = col("--background") { t.bg = c; }
        if let Some(c) = col("--background2") { t.bg2 = c; }
        if let Some(c) = col("--accent") { t.accent = c; t.warn = c; }
        if let Some(c) = col("--text") { t.text = c; }
        if let Some(c) = col("--subtext") { t.subtext = c; }
        if let Some(c) = col("--border") { t.border = c; }
        if let Some(c) = col("--hover") { t.hover = c; }
        // Only if the theme's "red" really is red (some themes set it to white).
        if let Some(c) = col("--redFlash").filter(|c| c.r > c.g + 0.15 && c.r > c.b + 0.1) { t.bad = c; }
        if let Some(f) = resolve(&vars, "--system-font").and_then(|v| first_font(&v)) {
            t.font = f;
        }
        if let Some(f) = resolve(&vars, "--icons-font").and_then(|v| first_font(&v)) {
            t.icon_font = f;
        }
        if let Ok(y) = std::fs::read_to_string(dir.join("config.yaml")) {
            let (h, top) = bar_geometry(&y);
            if let Some(h) = h { t.bar_height = h; }
            if let Some(top) = top { t.bar_top = top; }
        }
        t
    }
}
