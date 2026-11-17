//! Events from YASB's clock calendar (`calendar-events.json`), with the same recurrence
//! rules YASB uses, plus timed reminders.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::SystemTime;

use serde::Deserialize;
use windows::Win32::System::SystemInformation::GetLocalTime;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub struct Date {
    pub y: i32,
    pub m: u32,
    pub d: u32,
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

impl Date {
    pub fn parse(s: &str) -> Option<Date> {
        let mut it = s.trim().splitn(3, '-');
        let y = it.next()?.parse().ok()?;
        let m = it.next()?.parse().ok()?;
        let d = it.next()?.get(..2)?.parse().ok()?;
        (1..=12).contains(&m).then_some(Date { y, m, d })
    }

    /// Days since 1970-01-01 (proleptic Gregorian).
    pub fn days(self) -> i64 {
        let y = if self.m <= 2 { self.y - 1 } else { self.y } as i64;
        let era = if y >= 0 { y } else { y - 399 } / 400;
        let yoe = y - era * 400;
        let m = self.m as i64;
        let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + self.d as i64 - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146097 + doe - 719468
    }

    pub fn from_days(z: i64) -> Date {
        let z = z + 719468;
        let era = if z >= 0 { z } else { z - 146096 } / 146097;
        let doe = z - era * 146097;
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
        let y = (yoe + era * 400) as i32 + i32::from(m <= 2);
        Date { y, m, d }
    }

    pub fn add(self, n: i64) -> Date {
        Date::from_days(self.days() + n)
    }

    pub fn weekday(self) -> usize {
        (self.days() + 4).rem_euclid(7) as usize
    }

    pub fn short(self) -> String {
        format!("{} {} {}", DAYS[self.weekday()], self.d, MONTHS[(self.m - 1) as usize])
    }
}

fn month_len(y: i32, m: u32) -> u32 {
    match m {
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

pub struct Now {
    pub date: Date,
    pub h: u32,
    pub min: u32,
    pub s: u32,
}

pub fn local_now() -> Now {
    let t = unsafe { GetLocalTime() };
    Now {
        date: Date { y: t.wYear as i32, m: t.wMonth as u32, d: t.wDay as u32 },
        h: t.wHour as u32,
        min: t.wMinute as u32,
        s: t.wSecond as u32,
    }
}

/// The pill's clock: ("Sun 15 Nov", "1:42 PM"), in 24-hour time if Windows is set to it.
pub fn clock_text() -> (String, String) {
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let t = unsafe { GetLocalTime() };
    let date = format!(
        "{} {} {}",
        DAYS[t.wDayOfWeek as usize % 7],
        t.wDay,
        MONTHS[(t.wMonth as usize).clamp(1, 12) - 1]
    );
    let time = if uses_24h() {
        format!("{:02}:{:02}", t.wHour, t.wMinute)
    } else {
        let h = match t.wHour % 12 { 0 => 12, h => h };
        format!("{h}:{:02} {}", t.wMinute, if t.wHour < 12 { "AM" } else { "PM" })
    };
    (date, time)
}

/// Milliseconds until the next minute starts (plus a little, so the clock never shows the old one).
pub fn ms_to_next_minute() -> u32 {
    let t = unsafe { GetLocalTime() };
    (60 - t.wSecond as u32) * 1000 - t.wMilliseconds as u32 + 50
}

/// Windows' "Short time" setting: `iTime` is "1" for 24-hour.
fn uses_24h() -> bool {
    use windows::core::w;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};
    let mut buf = [0u16; 4];
    let mut n = std::mem::size_of_val(&buf) as u32;
    let ok = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Control Panel\\International"),
            w!("iTime"),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr() as *mut std::ffi::c_void),
            Some(&mut n),
        )
    };
    ok.is_ok() && buf[0] == '1' as u16
}

#[derive(Deserialize, Default, Clone)]
struct Recurrence {
    #[serde(default)]
    unit: String,
    #[serde(default)]
    interval: Option<i64>,
    #[serde(default)]
    days: Vec<u32>,
}

#[derive(Deserialize, Clone)]
struct Ev {
    #[serde(default)]
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    date: String,
    #[serde(default)]
    time: String,
    #[serde(default)]
    recurrence: Option<Recurrence>,
}

fn occurs_on(ev: &Ev, target: Date) -> bool {
    let Some(start) = Date::parse(&ev.date) else { return false };
    if target < start {
        return false;
    }
    let rec = ev.recurrence.clone().unwrap_or_default();
    let unit = if rec.unit.is_empty() { "once" } else { rec.unit.as_str() };
    let interval = rec.interval.unwrap_or(1).max(1);
    let delta = target.days() - start.days();
    let months = (target.y - start.y) as i64 * 12 + target.m as i64 - start.m as i64;
    match unit {
        "once" => target == start,
        "day" => delta % interval == 0,
        "week" => delta % (interval * 7) == 0,
        "month" => months % interval == 0 && target.d == start.d.min(month_len(target.y, target.m)),
        "month_days" => months % interval == 0 && rec.days.contains(&target.d),
        "year" => {
            let years = (target.y - start.y) as i64;
            years % interval == 0 && target.m == start.m && target.d == start.d.min(month_len(target.y, start.m))
        }
        _ => false,
    }
}

fn parse_hm(s: &str) -> Option<u32> {
    let (h, m) = s.trim().split_once(':')?;
    let (h, m): (u32, u32) = (h.parse().ok()?, m.get(..2)?.parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

pub struct Calendar {
    path: PathBuf,
    mtime: Option<SystemTime>,
    events: Vec<Ev>,
    fired: HashSet<(String, Date)>,
}

impl Calendar {
    pub fn new() -> Self {
        let mut c = Self {
            path: crate::util::calendar_file(),
            mtime: None,
            events: Vec::new(),
            fired: HashSet::new(),
        };
        c.reload();
        c
    }

    fn reload(&mut self) {
        let mtime = std::fs::metadata(&self.path).and_then(|m| m.modified()).ok();
        if mtime == self.mtime && mtime.is_some() {
            return;
        }
        self.mtime = mtime;
        self.events = std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
    }

    pub fn on(&mut self, d: Date) -> Vec<String> {
        self.reload();
        let mut v: Vec<(u32, String)> = self
            .events
            .iter()
            .filter(|e| occurs_on(e, d))
            .map(|e| {
                let t = parse_hm(&e.time);
                let label = match t {
                    Some(t) => format!("{:02}:{:02} {}", t / 60, t % 60, e.title),
                    None => e.title.clone(),
                };
                (t.unwrap_or(0), label)
            })
            .collect();
        v.sort();
        v.into_iter().map(|(_, s)| s).collect()
    }

    /// The first event after `today`, within `horizon` days.
    pub fn next_after(&mut self, today: Date, horizon: i64) -> Option<(String, Date)> {
        for n in 1..=horizon {
            let d = today.add(n);
            if let Some(first) = self.on(d).into_iter().next() {
                return Some((first, d));
            }
        }
        None
    }

    /// Timed events whose reminder time has come and hasn't been shown yet.
    pub fn due(&mut self, now: &Now, lead_min: u32) -> Vec<(String, String)> {
        self.reload();
        let mins = now.h * 60 + now.min;
        let mut out = Vec::new();
        for e in self.events.iter().filter(|e| occurs_on(e, now.date)) {
            let Some(t) = parse_hm(&e.time) else { continue };
            let remind_at = t.saturating_sub(lead_min);
            let key = (if e.id.is_empty() { e.title.clone() } else { e.id.clone() }, now.date);
            if mins >= remind_at && mins <= t && self.fired.insert(key) {
                out.push((e.title.clone(), format!("{:02}:{:02}", t / 60, t % 60)));
            }
        }
        out
    }

    /// The soonest timed event today starting within `within` minutes (or this minute):
    /// its title and the minutes left.
    pub fn upcoming(&mut self, now: &Now, within: u32) -> Option<(String, u32)> {
        self.reload();
        let mins = now.h * 60 + now.min;
        self.events
            .iter()
            .filter(|e| occurs_on(e, now.date))
            .filter_map(|e| {
                let t = parse_hm(&e.time)?;
                (t >= mins && t - mins <= within).then(|| (e.title.clone(), t - mins))
            })
            .min_by_key(|(_, left)| *left)
    }

    /// Milliseconds until the next reminder, the next midnight, or one hour, whichever is first.
    pub fn next_wake_ms(&mut self, now: &Now, lead_min: u32) -> u32 {
        self.reload();
        let secs_now = now.h * 3600 + now.min * 60 + now.s;
        let mut best = 86_400 - secs_now;
        for e in self.events.iter().filter(|e| occurs_on(e, now.date)) {
            if let Some(t) = parse_hm(&e.time) {
                let at = t.saturating_sub(lead_min) * 60;
                if at > secs_now {
                    best = best.min(at - secs_now);
                }
            }
        }
        best.clamp(5, 3600) * 1000 + 500
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_round_trip() {
        for z in [-1000i64, 0, 1, 20_000, 20_407, 21_000] {
            assert_eq!(Date::from_days(z).days(), z);
        }
        assert_eq!(Date { y: 2026, m: 11, d: 15 }.weekday(), 0); // a Sunday
    }

    #[test]
    fn yearly_and_monthly() {
        let ev = |date: &str, unit: &str| Ev {
            id: String::new(),
            title: "x".into(),
            date: date.into(),
            time: String::new(),
            recurrence: Some(Recurrence { unit: unit.into(), interval: Some(1), days: vec![] }),
        };
        assert!(occurs_on(&ev("2026-10-09", "year"), Date { y: 2027, m: 10, d: 9 }));
        assert!(!occurs_on(&ev("2026-10-09", "year"), Date { y: 2027, m: 10, d: 10 }));
        assert!(occurs_on(&ev("2026-01-31", "month"), Date { y: 2026, m: 2, d: 28 }));
        assert!(occurs_on(&ev("2026-11-01", "week"), Date { y: 2026, m: 11, d: 15 }));
    }
}
