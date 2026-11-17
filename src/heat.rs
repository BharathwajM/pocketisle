//! Heat warning: a peek when the CPU or GPU has been running hot for a while, naming the app
//! doing most of the work.
//!
//! The GPU's temperature comes from NVML (NVIDIA's driver). Windows can't read the CPU's
//! temperature without a kernel driver, so for the CPU the sign is Windows holding it below
//! full speed (thermal or power throttling) while it's busy, or it running flat out for five
//! minutes. Checked every 5 seconds; the work to find the app only happens once it's hot.

use std::time::{Duration, Instant};

use crate::events::{self, Event};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Part {
    Cpu,
    Gpu,
}

/// What a heat peek says.
#[derive(Clone, Debug, PartialEq)]
pub struct Heat {
    pub part: Part,
    /// The GPU's temperature (°C).
    pub temp: Option<u32>,
    /// How much of its full speed the CPU is allowed (percent), when it's held back.
    pub limit: Option<u32>,
    /// Load (0..1).
    pub load: f32,
    /// How long it's been like this.
    pub secs: u64,
    /// The app doing most of the work, and its share (0..1).
    pub app: Option<(String, f32)>,
}

const TICK: Duration = Duration::from_secs(5);
/// Hot this long before a peek: GPU over the line, CPU held back while busy, CPU flat out.
const GPU_SECS: u64 = 60;
const THROTTLE_SECS: u64 = 60;
const BUSY_SECS: u64 = 300;
/// No second peek about the same part within this long.
const QUIET: Duration = Duration::from_secs(30 * 60);

/// One part's state: hot since when; warned (waiting for it to cool before warning again).
#[derive(Default)]
struct Streak {
    since: Option<Instant>,
    warned: bool,
    last: Option<Instant>,
}

impl Streak {
    /// Feed one sample. Returns how long it's been hot when it's time to warn.
    fn step(&mut self, hot: bool, cool: bool, need: u64, now: Instant) -> Option<u64> {
        if cool {
            self.since = None;
            self.warned = false;
            return None;
        }
        if !hot {
            // In between: neither hot nor cooled down. The clock starts again.
            self.since = None;
            return None;
        }
        let since = *self.since.get_or_insert(now);
        let held = now.duration_since(since).as_secs();
        let quiet = self.last.is_some_and(|t| now.duration_since(t) < QUIET);
        if held >= need && !self.warned && !quiet {
            self.warned = true;
            self.last = Some(now);
            return Some(held);
        }
        None
    }

    fn building(&self) -> bool {
        self.since.is_some() && !self.warned
    }
}

/// Is the GPU too hot? Over the line is hot; 5° under it is cooled down.
pub fn gpu_state(temp: u32, line: u32) -> (bool, bool) {
    (temp >= line, temp + 5 < line)
}

/// The CPU: held back (under 90% of full speed) while at least 40% busy; flat out (95%+);
/// cooled down when under half busy and allowed (nearly) full speed.
pub fn cpu_state(load: f32, limit: Option<u32>) -> (bool, bool, bool) {
    let held = limit.is_some_and(|l| l < 90);
    let throttled = held && load >= 0.4;
    let busy = load >= 0.95;
    let cool = load < 0.5 && !held;
    (throttled, busy, cool)
}

/// "pid_1234_luid_0x0_0x1_phys_0_eng_0_engtype_3D" → 1234.
pub fn pid_of(instance: &str) -> Option<u32> {
    instance.strip_prefix("pid_")?.split('_').next()?.parse().ok()
}

/// Add up each process's GPU use from the engine counters (every engine of a process).
pub fn gpu_by_pid(items: &[(String, f64)]) -> Vec<(u32, f64)> {
    let mut by: Vec<(u32, f64)> = Vec::new();
    for (name, v) in items {
        let Some(pid) = pid_of(name) else { continue };
        match by.iter_mut().find(|(p, _)| *p == pid) {
            Some(e) => e.1 += v,
            None => by.push((pid, *v)),
        }
    }
    by.sort_by(|a, b| b.1.total_cmp(&a.1));
    by
}

pub fn spawn(gpu_line: u32) {
    let _ = std::thread::Builder::new().name("heat".into()).spawn(move || run(gpu_line.clamp(60, 105)));
}

fn run(gpu_line: u32) {
    // Give start-up a moment.
    std::thread::sleep(Duration::from_secs(30));
    let nvml = crate::sysinfo::Nvml::load();
    let mut cpu_q = pdh::Query::new();
    let limit_c = cpu_q.as_mut().and_then(|q| q.add("\\Processor Information(_Total)\\% Performance Limit"));
    let mut gpu_q: Option<(pdh::Query, pdh::Counter)> = None;
    let mut sampler: Option<crate::procs::Sampler> = None;
    let mut last = crate::sysinfo::cpu_times();
    let (mut gpu, mut throttle, mut busy) = (Streak::default(), Streak::default(), Streak::default());
    loop {
        std::thread::sleep(TICK);
        let now = Instant::now();
        // The CPU's load since the last look, and how much of full speed it's allowed.
        let mut load = 0.0f32;
        if let (Some((i0, t0)), Some((i1, t1))) = (last, crate::sysinfo::cpu_times()) {
            let dt = t1.saturating_sub(t0);
            if dt > 0 {
                load = (1.0 - i1.saturating_sub(i0) as f64 / dt as f64).clamp(0.0, 1.0) as f32;
            }
            last = Some((i1, t1));
        }
        let limit = match (cpu_q.as_mut(), limit_c) {
            (Some(q), Some(c)) => {
                q.collect();
                q.value(c).map(|v| v.round().clamp(0.0, 200.0) as u32)
            }
            _ => None,
        };
        let (hot_t, hot_b, cool) = cpu_state(load, limit);
        let t_fire = throttle.step(hot_t, cool, THROTTLE_SECS, now);
        let b_fire = busy.step(hot_b, cool, BUSY_SECS, now);
        // Which app: sample processes only while the CPU is heading for a warning.
        if throttle.building() || busy.building() || t_fire.is_some() || b_fire.is_some() {
            let s = sampler.get_or_insert_with(Default::default);
            let top = s.sample();
            if let Some(secs) = t_fire.or(b_fire) {
                let app = top.first().filter(|p| p.cpu >= 0.05).map(|p| (p.name.clone(), p.cpu));
                let limit = if t_fire.is_some() { limit } else { None };
                crate::util::log(&format!("heat: CPU {} for {secs} s", if limit.is_some() { "held back" } else { "flat out" }));
                events::send(Event::Heat(Heat { part: Part::Cpu, temp: None, limit, load, secs, app }));
                // Both streaks count as warned, so it's one peek.
                throttle.warned = true;
                busy.warned = true;
                throttle.last = Some(now);
                busy.last = Some(now);
            }
        } else {
            sampler = None;
        }

        // The GPU.
        let Some(g) = nvml.as_ref().and_then(|n| n.sample()) else { continue };
        let Some(temp) = g.temp else { continue };
        let (hot, cool) = gpu_state(temp, gpu_line);
        let fire = gpu.step(hot, cool, GPU_SECS, now);
        if gpu.building() || fire.is_some() {
            if gpu_q.is_none() {
                gpu_q = pdh::Query::new().and_then(|mut q| q.add("\\GPU Engine(*)\\Utilization Percentage").map(|c| (q, c)));
            }
            if let Some((q, c)) = gpu_q.as_mut() {
                q.collect();
                if let Some(secs) = fire {
                    let by = gpu_by_pid(&q.array(*c));
                    let total: f64 = by.iter().map(|(_, v)| v).sum::<f64>().max(1.0);
                    let app = by.iter().find_map(|(pid, v)| crate::procs::name_of(*pid).map(|n| (n, (*v / total) as f32)));
                    crate::util::log(&format!("heat: GPU at {temp}°C for {secs} s"));
                    events::send(Event::Heat(Heat { part: Part::Gpu, temp: Some(temp), limit: None, load: g.util, secs, app }));
                }
            }
        } else {
            gpu_q = None;
        }
    }
}

/// Windows performance counters (pdh.dll), just what's needed here.
mod pdh {
    #[link(name = "pdh")]
    extern "system" {
        fn PdhOpenQueryW(src: *const u16, user: usize, q: *mut isize) -> u32;
        fn PdhAddEnglishCounterW(q: isize, path: *const u16, user: usize, c: *mut isize) -> u32;
        fn PdhCollectQueryData(q: isize) -> u32;
        fn PdhGetFormattedCounterValue(c: isize, fmt: u32, ty: *mut u32, v: *mut Value) -> u32;
        fn PdhGetFormattedCounterArrayW(c: isize, fmt: u32, size: *mut u32, count: *mut u32, items: *mut u8) -> u32;
        fn PdhCloseQuery(q: isize) -> u32;
    }

    const FMT_DOUBLE: u32 = 0x0000_0200;
    const FMT_NOCAP100: u32 = 0x0000_8000;
    const MORE_DATA: u32 = 0x8000_07D2;

    /// PDH_FMT_COUNTERVALUE: a status, then (8-aligned) the value.
    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct Value {
        status: u32,
        value: f64,
    }

    /// PDH_FMT_COUNTERVALUE_ITEM_W.
    #[repr(C)]
    struct Item {
        name: *const u16,
        value: Value,
    }

    pub struct Query(isize);
    #[derive(Clone, Copy)]
    pub struct Counter(isize);

    unsafe impl Send for Query {}

    impl Query {
        pub fn new() -> Option<Query> {
            let mut q = 0isize;
            (unsafe { PdhOpenQueryW(std::ptr::null(), 0, &mut q) } == 0).then_some(Query(q))
        }

        pub fn add(&mut self, path: &str) -> Option<Counter> {
            let w: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
            let mut c = 0isize;
            let ok = unsafe { PdhAddEnglishCounterW(self.0, w.as_ptr(), 0, &mut c) } == 0;
            if ok {
                // Rates need two samples: take the first now.
                self.collect();
            }
            ok.then_some(Counter(c))
        }

        pub fn collect(&mut self) {
            unsafe {
                PdhCollectQueryData(self.0);
            }
        }

        pub fn value(&self, c: Counter) -> Option<f64> {
            let mut v = Value::default();
            let st = unsafe { PdhGetFormattedCounterValue(c.0, FMT_DOUBLE | FMT_NOCAP100, std::ptr::null_mut(), &mut v) };
            (st == 0 && v.status == 0).then_some(v.value)
        }

        /// Every instance of a wildcard counter: (instance name, value).
        pub fn array(&self, c: Counter) -> Vec<(String, f64)> {
            let fmt = FMT_DOUBLE | FMT_NOCAP100;
            let (mut size, mut count) = (0u32, 0u32);
            unsafe {
                if PdhGetFormattedCounterArrayW(c.0, fmt, &mut size, &mut count, std::ptr::null_mut()) != MORE_DATA || size == 0 {
                    return Vec::new();
                }
                // 8-aligned room for the items and the names after them.
                let mut buf = vec![0u64; (size as usize).div_ceil(8) + 1];
                if PdhGetFormattedCounterArrayW(c.0, fmt, &mut size, &mut count, buf.as_mut_ptr() as *mut u8) != 0 {
                    return Vec::new();
                }
                let items = std::slice::from_raw_parts(buf.as_ptr() as *const Item, count as usize);
                items
                    .iter()
                    .filter(|i| i.value.status == 0 && !i.name.is_null())
                    .map(|i| {
                        let mut n = 0;
                        while *i.name.add(n) != 0 {
                            n += 1;
                        }
                        (String::from_utf16_lossy(std::slice::from_raw_parts(i.name, n)), i.value.value)
                    })
                    .collect()
            }
        }
    }

    impl Drop for Query {
        fn drop(&mut self) {
            unsafe {
                PdhCloseQuery(self.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hot_and_cool() {
        assert_eq!(gpu_state(87, 85), (true, false));
        assert_eq!(gpu_state(82, 85), (false, false));
        assert_eq!(gpu_state(79, 85), (false, true));
        // Held back while busy.
        assert_eq!(cpu_state(0.6, Some(70)), (true, false, false));
        // Held back but idle: not a warning (and not cooled down either).
        assert_eq!(cpu_state(0.1, Some(70)), (false, false, false));
        assert_eq!(cpu_state(0.98, Some(100)), (false, true, false));
        assert_eq!(cpu_state(0.2, Some(100)), (false, false, true));
    }

    #[test]
    fn streaks_wait_then_warn_once() {
        let t0 = Instant::now();
        let mut s = Streak::default();
        assert_eq!(s.step(true, false, 60, t0), None);
        assert_eq!(s.step(true, false, 60, t0 + Duration::from_secs(30)), None);
        assert_eq!(s.step(true, false, 60, t0 + Duration::from_secs(60)), Some(60));
        // Still hot: no second peek.
        assert_eq!(s.step(true, false, 60, t0 + Duration::from_secs(200)), None);
        // Cooled down, hot again soon after: still quiet for half an hour.
        s.step(false, true, 60, t0 + Duration::from_secs(300));
        assert_eq!(s.step(true, false, 60, t0 + Duration::from_secs(400)), None);
        assert_eq!(s.step(true, false, 60, t0 + Duration::from_secs(500)), None);
        // An hour on, hot for a minute again: a new peek.
        s.step(false, true, 60, t0 + Duration::from_secs(3600));
        s.step(true, false, 60, t0 + Duration::from_secs(3700));
        assert_eq!(s.step(true, false, 60, t0 + Duration::from_secs(3760)), Some(60));
    }

    #[test]
    fn gpu_engines_by_app() {
        let items = vec![
            ("pid_100_luid_0x0_0x1_phys_0_eng_0_engtype_3D".to_string(), 50.0),
            ("pid_100_luid_0x0_0x1_phys_0_eng_3_engtype_Compute".to_string(), 10.0),
            ("pid_7_luid_0x0_0x1_phys_0_eng_0_engtype_3D".to_string(), 5.0),
            ("not_a_pid".to_string(), 99.0),
        ];
        assert_eq!(gpu_by_pid(&items), vec![(100, 60.0), (7, 5.0)]);
    }

    /// What the counters say on this PC (run by hand: cargo test heat_here -- --ignored --nocapture).
    #[test]
    #[ignore]
    fn heat_here() {
        let mut q = pdh::Query::new().expect("pdh");
        let lim = q.add("\\Processor Information(_Total)\\% Performance Limit");
        let gpu = q.add("\\GPU Engine(*)\\Utilization Percentage");
        std::thread::sleep(Duration::from_secs(1));
        q.collect();
        println!("limit: {:?}", lim.and_then(|c| q.value(c)));
        if let Some(c) = gpu {
            for (pid, v) in gpu_by_pid(&q.array(c)).into_iter().take(3) {
                println!("gpu: {:?} {v:.1}", crate::procs::name_of(pid));
            }
        }
        println!("nvml temp: {:?}", crate::sysinfo::Nvml::load().and_then(|n| n.sample()).and_then(|g| g.temp));
    }
}
