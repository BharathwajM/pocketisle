//! Weather from Open-Meteo (free, no key), for the place YASB's weather widget was set to
//! (`%LOCALAPPDATA%\YASB\weather.json`) unless `weather_location` is set in Pocket Isle's settings.

use std::time::Duration;

use serde_json::Value;

use crate::events::{self, Event};
use crate::util;

#[derive(Clone)]
pub struct Hour {
    pub label: String,
    pub temp: f32,
    pub code: i64,
    #[allow(dead_code)]
    pub rain: i64,
    pub day: bool,
}

#[derive(Clone)]
pub struct Day {
    pub label: String,
    pub code: i64,
    pub hi: f32,
    pub lo: f32,
    pub rain: i64,
}

#[derive(Clone)]
pub struct Weather {
    pub place: String,
    pub temp: f32,
    pub feels: f32,
    pub code: i64,
    pub day: bool,
    pub humidity: i64,
    pub wind: f32,
    pub hours: Vec<Hour>,
    pub days: Vec<Day>,
}

#[derive(Clone, PartialEq)]
pub struct Place {
    pub lat: f64,
    pub lon: f64,
    pub name: String,
}

/// The location saved by YASB's Open-Meteo widget.
pub fn yasb_place() -> Option<Place> {
    let local = std::env::var_os("LOCALAPPDATA")?;
    let text = std::fs::read_to_string(std::path::Path::new(&local).join(r"YASB\weather.json")).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    let entry = v.as_object()?.values().find(|e| e["latitude"].is_number())?;
    Some(Place {
        lat: entry["latitude"].as_f64()?,
        lon: entry["longitude"].as_f64()?,
        name: entry["name"].as_str().unwrap_or("").to_string(),
    })
}

pub fn spawn(place: Option<Place>, every_minutes: u32) {
    let Some(place) = place else {
        util::log("weather: no location (set one in YASB's weather widget or weather_location)");
        return;
    };
    let every = Duration::from_secs(every_minutes.clamp(5, 180) as u64 * 60);
    std::thread::Builder::new()
        .name("weather".into())
        .spawn(move || loop {
            let wait = match fetch(&place) {
                Ok(w) => {
                    events::send(Event::Weather(w));
                    every
                }
                Err(e) => {
                    util::log(&format!("weather: {e}"));
                    // Meanwhile, the last forecast's hour for now (up to a day old).
                    if let Some(w) = cached(&place) {
                        events::send(Event::Weather(w));
                    }
                    Duration::from_secs(120)
                }
            };
            std::thread::sleep(wait);
        })
        .ok();
}

fn fetch(p: &Place) -> Result<Weather, String> {
    let path = format!(
        "/v1/forecast?latitude={:.4}&longitude={:.4}\
         &current=temperature_2m,apparent_temperature,relative_humidity_2m,weather_code,is_day,wind_speed_10m\
         &hourly=temperature_2m,weather_code,precipitation_probability,is_day\
         &daily=weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max\
         &timezone=auto&forecast_days=6",
        p.lat, p.lon
    );
    let (status, body) = crate::usage::https_get("api.open-meteo.com", &path, String::from("Accept: application/json\r\n"))?;
    if status != 200 {
        return Err(format!("HTTP {status}"));
    }
    let v: Value = serde_json::from_slice(&body).map_err(|_| "unreadable response".to_string())?;
    let w = parse(&v, &p.name).ok_or_else(|| "unexpected response".to_string())?;
    let _ = std::fs::write(cache_path(), &body);
    Ok(w)
}

/// The last forecast, kept for when Open-Meteo can't be reached.
fn cache_path() -> std::path::PathBuf {
    util::config_dir().join("weather_cache.json")
}

/// The saved forecast's hour for now, if it's less than a day old.
fn cached(p: &Place) -> Option<Weather> {
    let path = cache_path();
    let age = std::fs::metadata(&path).ok()?.modified().ok()?.elapsed().ok()?;
    if age > Duration::from_secs(24 * 3600) {
        return None;
    }
    let v: Value = serde_json::from_slice(&std::fs::read(&path).ok()?).ok()?;
    let now = crate::calendar::local_now();
    let key = format!("{:04}-{:02}-{:02}T{:02}:00", now.date.y, now.date.m, now.date.d, now.h);
    from_forecast(&v, &p.name, &key)
}

/// A saved forecast as if it were the hour `key` ("2026-11-15T21:00"): that hour's sky,
/// temperature and daylight become the current weather.
fn from_forecast(v: &Value, place: &str, key: &str) -> Option<Weather> {
    let hourly = &v["hourly"];
    let i = hourly["time"].as_array()?.iter().position(|t| t.as_str() == Some(key))?;
    let mut now = v.clone();
    let c = &mut now["current"];
    c["time"] = Value::from(key);
    c["weather_code"] = hourly["weather_code"][i].clone();
    c["temperature_2m"] = hourly["temperature_2m"][i].clone();
    c["apparent_temperature"] = hourly["temperature_2m"][i].clone();
    c["is_day"] = hourly["is_day"][i].clone();
    parse(&now, place)
}

fn f(v: &Value) -> f32 {
    v.as_f64().unwrap_or(0.0) as f32
}

const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

fn weekday(date: &str) -> Option<&'static str> {
    let y: i64 = date.get(0..4)?.parse().ok()?;
    let m: i64 = date.get(5..7)?.parse().ok()?;
    let d: i64 = date.get(8..10)?.parse().ok()?;
    let d = crate::calendar::Date { y: y as i32, m: m as u32, d: d as u32 };
    Some(WEEKDAYS[d.weekday() % 7])
}

fn parse(v: &Value, place: &str) -> Option<Weather> {
    let c = &v["current"];
    let now = c["time"].as_str()?;
    let hourly = &v["hourly"];
    let times = hourly["time"].as_array()?;
    // First hour after the current one.
    let start = times.iter().position(|t| t.as_str().map(|t| t > now).unwrap_or(false)).unwrap_or(0);
    let hours = (start..(start + 6).min(times.len()))
        .map(|i| Hour {
            label: times[i].as_str().and_then(|t| t.get(11..16)).unwrap_or("").to_string(),
            temp: f(&hourly["temperature_2m"][i]),
            code: hourly["weather_code"][i].as_i64().unwrap_or(0),
            rain: hourly["precipitation_probability"][i].as_i64().unwrap_or(0),
            day: hourly["is_day"][i].as_i64().unwrap_or(1) == 1,
        })
        .collect();
    let daily = &v["daily"];
    let days = daily["time"]
        .as_array()?
        .iter()
        .enumerate()
        .map(|(i, t)| Day {
            label: if i == 0 { "Today".into() } else { t.as_str().and_then(weekday).unwrap_or("").to_string() },
            code: daily["weather_code"][i].as_i64().unwrap_or(0),
            hi: f(&daily["temperature_2m_max"][i]),
            lo: f(&daily["temperature_2m_min"][i]),
            rain: daily["precipitation_probability_max"][i].as_i64().unwrap_or(0),
        })
        .collect();
    Some(Weather {
        place: place.to_string(),
        temp: f(&c["temperature_2m"]),
        feels: f(&c["apparent_temperature"]),
        code: c["weather_code"].as_i64().unwrap_or(0),
        day: c["is_day"].as_i64().unwrap_or(1) == 1,
        humidity: c["relative_humidity_2m"].as_i64().unwrap_or(0),
        wind: f(&c["wind_speed_10m"]),
        hours,
        days,
    })
}

/// WMO weather code -> (Nerd Font glyph, description).
pub fn describe(code: i64, day: bool) -> (&'static str, &'static str) {
    match code {
        0 => (if day { "\u{f0599}" } else { "\u{f0594}" }, "Clear"),
        1 => (if day { "\u{f0599}" } else { "\u{f0594}" }, "Mostly clear"),
        2 => (if day { "\u{f0595}" } else { "\u{f0f31}" }, "Partly cloudy"),
        3 => ("\u{f0590}", "Overcast"),
        45 | 48 => ("\u{f0591}", "Fog"),
        51 | 53 | 55 | 56 | 57 => ("\u{f0597}", "Drizzle"),
        61 | 63 | 66 => ("\u{f0597}", "Rain"),
        65 | 67 => ("\u{f0596}", "Heavy rain"),
        71 | 73 | 75 | 77 => ("\u{f0598}", "Snow"),
        80 | 81 => ("\u{f0597}", "Showers"),
        82 => ("\u{f0596}", "Heavy showers"),
        85 | 86 => ("\u{f0598}", "Snow showers"),
        95 => ("\u{f067e}", "Thunderstorm"),
        96 | 99 => ("\u{f067e}", "Thunderstorm with hail"),
        _ => ("\u{f0590}", "Cloudy"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sample() {
        let v: Value = serde_json::from_str(r#"{
            "current":{"time":"2026-11-15T09:00","temperature_2m":20.3,"apparent_temperature":20.2,"relative_humidity_2m":78,"weather_code":3,"is_day":1,"wind_speed_10m":15.2},
            "hourly":{"time":["2026-11-15T08:00","2026-11-15T09:00","2026-11-15T10:00","2026-11-15T11:00"],
                      "temperature_2m":[19.8,20.3,20.7,21.6],"weather_code":[3,3,51,3],"precipitation_probability":[22,32,47,59],"is_day":[1,1,1,1]},
            "daily":{"time":["2026-11-15","2026-11-16"],"weather_code":[51,53],"temperature_2m_max":[21.8,17.8],"temperature_2m_min":[17.6,15.9],"precipitation_probability_max":[82,97]}
        }"#).unwrap();
        let w = parse(&v, "Sydney").unwrap();
        assert_eq!(w.hours.len(), 2);
        assert_eq!(w.hours[0].label, "10:00");
        assert_eq!(w.days[1].label, "Mon");
        assert_eq!(describe(w.code, w.day).1, "Overcast");
        // Offline: the saved forecast's hour stands in for now.
        let later = from_forecast(&v, "Sydney", "2026-11-15T10:00").unwrap();
        assert_eq!((later.code, later.temp), (51, 20.7));
        assert_eq!(later.hours[0].label, "11:00");
        assert!(from_forecast(&v, "Sydney", "2026-11-20T10:00").is_none());
    }
}
