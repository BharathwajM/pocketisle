//! CPU, memory, GPU (NVIDIA, through the driver's NVML library) and C: drive, sampled once a
//! second, but only while the System tab is open. The rest of the time the thread is parked,
//! and it never wakes the laptop's discrete GPU on its own.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

use windows::core::{s, w, PCSTR};
use windows::Win32::Foundation::{FILETIME, HMODULE};
use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::System::Threading::GetSystemTimes;

use crate::events::{self, Event};

#[derive(Clone, Default)]
pub struct Gpu {
    pub name: String,
    pub util: f32,
    pub vram_used: u64,
    pub vram_total: u64,
    pub temp: Option<u32>,
    /// Memory controller load (0..1), clocks (MHz), power (W) and the driver.
    pub mem_util: f32,
    pub clock: Option<u32>,
    pub mem_clock: Option<u32>,
    pub power: Option<f32>,
    pub driver: String,
}

/// The processor: its name, base speed, cores and threads (read once).
#[derive(Clone, Default)]
pub struct CpuInfo {
    pub name: String,
    pub base_mhz: u32,
    pub cores: u32,
    pub threads: u32,
}

/// The memory and kernel counts behind Task Manager's Performance tab.
#[derive(Clone, Default)]
pub struct Perf {
    pub commit: u64,
    pub commit_limit: u64,
    pub cached: u64,
    pub paged: u64,
    pub nonpaged: u64,
    pub processes: u32,
    pub threads: u32,
    pub handles: u32,
}

/// A drive: "C:", its name, free and total bytes, and whether it's removable.
#[derive(Clone, Default)]
pub struct Drive {
    pub root: String,
    pub label: String,
    pub free: u64,
    pub total: u64,
    pub removable: bool,
    /// The disk it's on: "Samsung SSD 990 PRO 2TB", and "NVMe SSD" / "SSD" / "Hard drive" / "USB".
    pub model: String,
    pub kind: String,
}

/// A memory stick (from the firmware's SMBIOS table).
#[derive(Clone, Default, Debug)]
pub struct MemModule {
    pub size_mb: u32,
    /// "DDR5", "LPDDR5"…
    pub kind: String,
    /// MT/s.
    pub speed: u32,
    pub maker: String,
    pub part: String,
}

/// The RAM: its sticks, and how many slots there are.
#[derive(Clone, Default, Debug)]
pub struct MemInfo {
    pub modules: Vec<MemModule>,
    pub slots: usize,
}

impl MemInfo {
    /// "Samsung DDR5-5600" (empty if the firmware doesn't say).
    pub fn name(&self) -> String {
        let Some(m) = self.modules.first() else { return String::new() };
        let mut t = m.kind.clone();
        if m.speed > 0 && !t.is_empty() {
            t = format!("{t}-{}", m.speed);
        }
        let makers: Vec<&str> = self.modules.iter().map(|m| m.maker.as_str()).filter(|s| !s.is_empty()).collect();
        let maker = if !makers.is_empty() && makers.iter().all(|s| *s == makers[0]) { makers[0] } else { "" };
        [maker, t.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" ")
    }

    /// "2 × 32 GB" (or "16 GB + 8 GB").
    pub fn sticks(&self) -> String {
        let gb = |mb: u32| if mb % 1024 == 0 { format!("{} GB", mb / 1024) } else { format!("{:.1} GB", mb as f32 / 1024.0) };
        match self.modules.as_slice() {
            [] => String::new(),
            [m] => gb(m.size_mb),
            ms if ms.iter().all(|m| m.size_mb == ms[0].size_mb) => format!("{} × {}", ms.len(), gb(ms[0].size_mb)),
            ms => ms.iter().map(|m| gb(m.size_mb)).collect::<Vec<_>>().join(" + "),
        }
    }
}

#[derive(Clone, Default)]
pub struct Sys {
    pub cpu: f32,
    pub mem_used: u64,
    pub mem_total: u64,
    pub gpu: Option<Gpu>,
    pub disk_free: u64,
    pub disk_total: u64,
    /// Bytes a second down / up, and a ping to 1.1.1.1.
    pub net_down: f64,
    pub net_up: f64,
    pub ping: Option<u32>,
    /// The apps using the most CPU and memory.
    pub procs: Vec<crate::procs::Proc>,
    /// Details for the System tab's CPU, Memory and Storage pages.
    pub cpu_info: CpuInfo,
    /// Each logical processor's load (0..1) and the current speed (MHz).
    pub cores: Vec<f32>,
    pub cpu_mhz: Option<u32>,
    pub uptime_secs: u64,
    pub perf: Perf,
    pub drives: Vec<Drive>,
    pub mem_info: MemInfo,
    /// Bluetooth devices' batteries (earbuds, mouse, controller…).
    pub batteries: Vec<crate::devices::Battery>,
}

static ACTIVE: AtomicBool = AtomicBool::new(false);
static THREAD: OnceLock<std::thread::Thread> = OnceLock::new();

/// Start or stop sampling (the System tab is open / closed).
pub fn set_active(on: bool) {
    let was = ACTIVE.swap(on, Ordering::SeqCst);
    if on && !was {
        if let Some(t) = THREAD.get() {
            t.unpark();
        }
    }
}

pub fn spawn() {
    let h = std::thread::Builder::new().name("sysinfo".into()).spawn(run);
    if let Ok(h) = h {
        let _ = THREAD.set(h.thread().clone());
    }
}

fn ft(f: FILETIME) -> u64 {
    ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64
}

pub(crate) fn cpu_times() -> Option<(u64, u64)> {
    let (mut idle, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
    unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)).ok()? };
    // Kernel time includes idle time.
    Some((ft(idle), ft(kernel) + ft(user)))
}

// ---------- Details ----------

fn reg_string(key: windows::core::PCWSTR, value: windows::core::PCWSTR) -> Option<String> {
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};
    let mut buf = [0u16; 256];
    let mut n = (buf.len() * 2) as u32;
    unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, key, value, RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut n)) }.ok().ok()?;
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]).trim().to_string())
}

fn reg_dword(key: windows::core::PCWSTR, value: windows::core::PCWSTR) -> Option<u32> {
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD};
    let mut v = 0u32;
    let mut n = 4u32;
    unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, key, value, RRF_RT_REG_DWORD, None, Some(&mut v as *mut u32 as *mut _), Some(&mut n)) }.ok().ok()?;
    Some(v)
}

fn cpu_info() -> CpuInfo {
    use windows::Win32::System::SystemInformation::{GetLogicalProcessorInformation, RelationProcessorCore, SYSTEM_LOGICAL_PROCESSOR_INFORMATION};
    let key = w!("HARDWARE\\DESCRIPTION\\System\\CentralProcessor\\0");
    let name = reg_string(key, w!("ProcessorNameString")).unwrap_or_else(|| "Processor".into());
    let name = name.replace("(R)", "").replace("(TM)", "").split_whitespace().collect::<Vec<_>>().join(" ");
    let base_mhz = reg_dword(key, w!("~MHz")).unwrap_or(0);
    let threads = std::thread::available_parallelism().map(|n| n.get() as u32).unwrap_or(1);
    let mut len = 0u32;
    unsafe {
        let _ = GetLogicalProcessorInformation(None, &mut len);
    }
    let n = len as usize / std::mem::size_of::<SYSTEM_LOGICAL_PROCESSOR_INFORMATION>();
    let mut v: Vec<SYSTEM_LOGICAL_PROCESSOR_INFORMATION> = vec![unsafe { std::mem::zeroed() }; n];
    let cores = if n > 0 && unsafe { GetLogicalProcessorInformation(Some(v.as_mut_ptr()), &mut len) }.is_ok() {
        v.iter().filter(|i| i.Relationship == RelationProcessorCore).count() as u32
    } else {
        threads
    };
    CpuInfo { name, base_mhz, cores, threads }
}

/// Each logical processor's (idle, total) time, from ntdll.
fn core_times() -> Vec<(u64, u64)> {
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct Info {
        idle: i64,
        kernel: i64,
        user: i64,
        dpc: i64,
        interrupt: i64,
        count: u32,
    }
    type Query = unsafe extern "system" fn(u32, *mut core::ffi::c_void, u32, *mut u32) -> i32;
    static QUERY: OnceLock<Option<Query>> = OnceLock::new();
    let q = *QUERY.get_or_init(|| unsafe {
        let lib = LoadLibraryW(w!("ntdll.dll")).ok()?;
        GetProcAddress(lib, s!("NtQuerySystemInformation")).map(|f| std::mem::transmute::<_, Query>(f))
    });
    let Some(q) = q else { return Vec::new() };
    let n = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).min(256);
    let mut v = vec![Info::default(); n];
    let mut ret = 0u32;
    let st = unsafe { q(8, v.as_mut_ptr() as *mut _, (n * std::mem::size_of::<Info>()) as u32, &mut ret) };
    if st != 0 {
        return Vec::new();
    }
    let got = ret as usize / std::mem::size_of::<Info>();
    v.truncate(got);
    // Kernel time includes idle time.
    v.iter().map(|i| (i.idle as u64, (i.kernel + i.user) as u64)).collect()
}

fn cpu_mhz(threads: u32) -> Option<u32> {
    use windows::Win32::System::Power::{CallNtPowerInformation, ProcessorInformation};
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct Ppi {
        number: u32,
        max: u32,
        current: u32,
        limit: u32,
        max_idle: u32,
        idle: u32,
    }
    let mut v = vec![Ppi::default(); threads.max(1) as usize];
    let st = unsafe { CallNtPowerInformation(ProcessorInformation, None, 0, Some(v.as_mut_ptr() as *mut _), (v.len() * std::mem::size_of::<Ppi>()) as u32) };
    if st.0 != 0 {
        return None;
    }
    let sum: u64 = v.iter().map(|p| p.current as u64).sum();
    (sum > 0).then(|| (sum / v.len() as u64) as u32)
}

fn perf() -> Perf {
    use windows::Win32::System::ProcessStatus::{K32GetPerformanceInfo, PERFORMANCE_INFORMATION};
    let mut p = PERFORMANCE_INFORMATION { cb: std::mem::size_of::<PERFORMANCE_INFORMATION>() as u32, ..Default::default() };
    if !unsafe { K32GetPerformanceInfo(&mut p, p.cb) }.as_bool() {
        return Perf::default();
    }
    let pg = p.PageSize as u64;
    Perf {
        commit: p.CommitTotal as u64 * pg,
        commit_limit: p.CommitLimit as u64 * pg,
        cached: p.SystemCache as u64 * pg,
        paged: p.KernelPaged as u64 * pg,
        nonpaged: p.KernelNonpaged as u64 * pg,
        processes: p.ProcessCount,
        threads: p.ThreadCount,
        handles: p.HandleCount,
    }
}

/// The fixed and removable drives with their names and space.
pub fn drives() -> Vec<Drive> {
    use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW};
    let mask = unsafe { GetLogicalDrives() };
    let mut out = Vec::new();
    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let root = format!("{}:\\", (b'A' + i as u8) as char);
        let wide: Vec<u16> = root.encode_utf16().chain([0]).collect();
        let p = windows::core::PCWSTR(wide.as_ptr());
        let kind = unsafe { GetDriveTypeW(p) };
        // 2 removable, 3 fixed (not network, CD or RAM drives).
        if kind != 2 && kind != 3 {
            continue;
        }
        let (mut free, mut total) = (0u64, 0u64);
        if unsafe { GetDiskFreeSpaceExW(p, Some(&mut free), Some(&mut total), None) }.is_err() || total == 0 {
            continue;
        }
        let mut name = [0u16; 128];
        let _ = unsafe { GetVolumeInformationW(p, Some(&mut name), None, None, None, None) };
        let n = name.iter().position(|&c| c == 0).unwrap_or(name.len());
        let label = String::from_utf16_lossy(&name[..n]);
        let label = if label.is_empty() { if kind == 2 { "USB drive".to_string() } else { "Local Disk".to_string() } } else { label };
        let root = root.trim_end_matches('\\').to_string();
        let (model, how) = disk_model(&root).unwrap_or_default();
        out.push(Drive { root, label, free, total, removable: kind == 2, model, kind: how });
    }
    out
}

// ---------- Hardware names ----------

mod raw {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    extern "system" {
        pub fn GetSystemFirmwareTable(provider: u32, id: u32, buf: *mut u8, size: u32) -> u32;
        pub fn CreateFileW(name: *const u16, access: u32, share: u32, sa: *mut c_void, disp: u32, flags: u32, template: *mut c_void) -> isize;
        pub fn DeviceIoControl(h: isize, code: u32, inb: *const c_void, insz: u32, outb: *mut c_void, outsz: u32, got: *mut u32, ov: *mut c_void) -> i32;
        pub fn CloseHandle(h: isize) -> i32;
    }
}

/// A device opened only to ask it things (no read or write access needed, so no admin).
struct Handle(isize);

impl Handle {
    fn open(path: &str) -> Option<Handle> {
        let wide: Vec<u16> = path.encode_utf16().chain([0]).collect();
        // Share read | write, OPEN_EXISTING.
        let h = unsafe { raw::CreateFileW(wide.as_ptr(), 0, 3, std::ptr::null_mut(), 3, 0, std::ptr::null_mut()) };
        if h == -1 || h == 0 { None } else { Some(Handle(h)) }
    }

    fn ioctl(&self, code: u32, input: &[u8], out: &mut [u8]) -> Option<usize> {
        let mut got = 0u32;
        let ok = unsafe {
            raw::DeviceIoControl(self.0, code, input.as_ptr() as *const _, input.len() as u32, out.as_mut_ptr() as *mut _, out.len() as u32, &mut got, std::ptr::null_mut())
        };
        (ok != 0).then_some(got as usize)
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            raw::CloseHandle(self.0);
        }
    }
}

const IOCTL_STORAGE_GET_DEVICE_NUMBER: u32 = 0x2D_1080;
const IOCTL_STORAGE_QUERY_PROPERTY: u32 = 0x2D_1400;

fn u32_at(b: &[u8], i: usize) -> u32 {
    b.get(i..i + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).unwrap_or(0)
}

fn cstr_at(b: &[u8], off: usize) -> String {
    if off == 0 || off >= b.len() {
        return String::new();
    }
    let end = b[off..].iter().position(|&c| c == 0).map(|n| off + n).unwrap_or(b.len());
    String::from_utf8_lossy(&b[off..end]).trim().to_string()
}

/// The model of the disk a drive letter is on, and what kind of disk it is.
pub fn disk_model(root: &str) -> Option<(String, String)> {
    let letter = root.trim_end_matches(['\\', ':']);
    let vol = Handle::open(&format!("\\\\.\\{letter}:"))?;
    // Which physical disk (falls back to asking the volume itself).
    let mut num = [0u8; 12];
    let disk = vol.ioctl(IOCTL_STORAGE_GET_DEVICE_NUMBER, &[], &mut num).and_then(|_| Handle::open(&format!("\\\\.\\PhysicalDrive{}", u32_at(&num, 4))));
    let dev = disk.as_ref().unwrap_or(&vol);
    // StorageDeviceProperty (0), PropertyStandardQuery.
    let mut q = [0u8; 12];
    let mut d = vec![0u8; 1024];
    dev.ioctl(IOCTL_STORAGE_QUERY_PROPERTY, &q, &mut d)?;
    let vendor = cstr_at(&d, u32_at(&d, 12) as usize);
    let product = cstr_at(&d, u32_at(&d, 16) as usize);
    let bus = u32_at(&d, 28);
    let mut model = if !vendor.is_empty() && !product.to_lowercase().starts_with(&vendor.to_lowercase()) { format!("{vendor} {product}") } else { product };
    model = model.split_whitespace().collect::<Vec<_>>().join(" ");
    // "SAMSUNG MZVL…" → "Samsung MZVL…".
    if let Some(first) = model.split(' ').next().map(str::to_string) {
        if first.len() > 3 && first.chars().all(|c| c.is_ascii_uppercase()) {
            let nice = format!("{}{}", &first[..1], first[1..].to_ascii_lowercase());
            model = model.replacen(&first, &nice, 1);
        }
    }
    if model.is_empty() {
        return None;
    }
    // StorageDeviceSeekPenaltyProperty (7): spinning disks have one.
    q[0] = 7;
    let mut sp = [0u8; 12];
    let spins = dev.ioctl(IOCTL_STORAGE_QUERY_PROPERTY, &q, &mut sp).filter(|&n| n >= 9).map(|_| sp[8] != 0);
    let kind = match (bus, spins) {
        (17, _) => "NVMe SSD",
        (7, _) => "USB",
        (12, _) | (13, _) => "SD card",
        (_, Some(false)) => "SSD",
        (_, Some(true)) => "Hard drive",
        _ => "",
    };
    Some((model, kind.to_string()))
}

/// Memory makers by JEDEC code (what some firmware writes instead of the name).
fn jedec(maker: &str) -> String {
    let m = maker.trim();
    let up = m.to_ascii_uppercase();
    let known = [
        ("80CE", "Samsung"),
        ("80AD", "SK hynix"),
        ("802C", "Micron"),
        ("859B", "Crucial"),
        ("0198", "Kingston"),
        ("04CD", "G.Skill"),
        ("029E", "Corsair"),
        ("04CB", "ADATA"),
        ("8551", "Qimonda"),
        ("4304", "Ramaxel"),
        ("0B03", "Patriot"),
        ("CB04", "ADATA"),
        ("CE00", "Samsung"),
        ("AD00", "SK hynix"),
        ("2C00", "Micron"),
    ];
    if let Some((_, n)) = known.iter().find(|(c, _)| up.trim_start_matches("0X").starts_with(c)) {
        return n.to_string();
    }
    if m.is_empty() || ["UNKNOWN", "UNDEFINED", "NOT SPECIFIED", "MANUFACTURER", "TO BE FILLED"].iter().any(|b| up.starts_with(b)) || up.chars().all(|c| c.is_ascii_hexdigit()) {
        return String::new();
    }
    m.replace(" Technology", "").replace(" Inc.", "").trim().to_string()
}

fn mem_kind(t: u8) -> &'static str {
    match t {
        0x12 => "DDR",
        0x13 => "DDR2",
        0x18 => "DDR3",
        0x1A => "DDR4",
        0x1B => "LPDDR",
        0x1C => "LPDDR2",
        0x1D => "LPDDR3",
        0x1E => "LPDDR4",
        0x22 => "DDR5",
        0x23 => "LPDDR5",
        _ => "",
    }
}

/// The RAM sticks, from the SMBIOS table (type 17 entries).
pub fn mem_info() -> MemInfo {
    let rsmb = u32::from_be_bytes(*b"RSMB");
    let n = unsafe { raw::GetSystemFirmwareTable(rsmb, 0, std::ptr::null_mut(), 0) };
    if n < 8 {
        return MemInfo::default();
    }
    let mut buf = vec![0u8; n as usize];
    if unsafe { raw::GetSystemFirmwareTable(rsmb, 0, buf.as_mut_ptr(), n) } == 0 {
        return MemInfo::default();
    }
    parse_smbios(&buf[8..])
}

fn parse_smbios(t: &[u8]) -> MemInfo {
    let u16_at = |s: &[u8], i: usize| s.get(i..i + 2).map(|b| u16::from_le_bytes([b[0], b[1]])).unwrap_or(0);
    let mut info = MemInfo::default();
    let mut i = 0;
    while i + 4 <= t.len() {
        let kind = t[i];
        let len = t[i + 1] as usize;
        if len < 4 || i + len > t.len() {
            break;
        }
        let s = &t[i..i + len];
        // The strings follow, ending with two zeros.
        let mut j = i + len;
        let mut strings = Vec::new();
        while j < t.len() {
            let end = t[j..].iter().position(|&c| c == 0).map(|k| j + k).unwrap_or(t.len());
            if end == j {
                break;
            }
            strings.push(String::from_utf8_lossy(&t[j..end]).trim().to_string());
            j = end + 1;
        }
        let next = if strings.is_empty() { i + len + 2 } else { j + 1 };
        let string = |k: usize| s.get(k).and_then(|&n| (n as usize).checked_sub(1)).and_then(|n| strings.get(n)).cloned().unwrap_or_default();
        if kind == 127 {
            break;
        }
        if kind == 17 && len >= 0x15 {
            info.slots += 1;
            let size = u16_at(s, 0x0C);
            let size_mb = match size {
                0 | 0xFFFF => 0,
                0x7FFF => u32_at(s, 0x1C) & 0x7FFF_FFFF,
                n if n & 0x8000 != 0 => ((n & 0x7FFF) as u32) / 1024,
                n => n as u32,
            };
            if size_mb > 0 {
                let mut speed = if len > 0x21 { u16_at(s, 0x20) as u32 } else { 0 };
                if speed == 0 || speed == 0xFFFF {
                    speed = u16_at(s, 0x15) as u32;
                }
                if speed == 0xFFFF {
                    speed = if len >= 0x58 { u32_at(s, 0x54) } else { 0 };
                }
                let part = if len > 0x1A { string(0x1A) } else { String::new() };
                let part = if part.to_ascii_uppercase().starts_with("UNKNOWN") || part.chars().all(|c| c == '0' || c == ' ') { String::new() } else { part };
                info.modules.push(MemModule {
                    size_mb,
                    kind: mem_kind(s[0x12]).to_string(),
                    speed,
                    maker: if len > 0x17 { jedec(&string(0x17)) } else { String::new() },
                    part,
                });
            }
        }
        i = next;
    }
    info
}

fn run() {
    let mut nvml: Option<Option<Nvml>> = None;
    let info = cpu_info();
    let ram = mem_info();
    let mut last_cores = core_times();
    let mut drives_now = Vec::new();
    let mut batteries = Vec::new();
    let mut last = cpu_times();
    let mut last_net: Option<(std::time::Instant, (u64, u64))> = None;
    let (mut ping, mut tick) = (None, 0u32);
    let mut sampler = crate::procs::Sampler::default();
    let mut procs = Vec::new();
    loop {
        if !ACTIVE.load(Ordering::SeqCst) {
            std::thread::park();
            last = cpu_times();
            last_cores = core_times();
            last_net = None;
            tick = 0;
            procs.clear();
            std::thread::sleep(Duration::from_millis(250));
            continue;
        }
        let mut s = Sys::default();
        if let (Some((i0, t0)), Some((i1, t1))) = (last, cpu_times()) {
            let dt = t1.saturating_sub(t0);
            if dt > 0 {
                s.cpu = (1.0 - i1.saturating_sub(i0) as f64 / dt as f64).clamp(0.0, 1.0) as f32;
            }
            last = Some((i1, t1));
        }
        let mut m = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
        if unsafe { GlobalMemoryStatusEx(&mut m) }.is_ok() {
            s.mem_total = m.ullTotalPhys;
            s.mem_used = m.ullTotalPhys.saturating_sub(m.ullAvailPhys);
        }
        let (mut avail, mut total) = (0u64, 0u64);
        if unsafe { GetDiskFreeSpaceExW(w!("C:\\"), Some(&mut avail), Some(&mut total), None) }.is_ok() {
            s.disk_free = avail;
            s.disk_total = total;
        }
        let n = nvml.get_or_insert_with(Nvml::load);
        s.gpu = n.as_ref().and_then(|n| n.sample());
        let at = std::time::Instant::now();
        let (rx, tx) = crate::net::totals();
        if let Some((t0, (r0, x0))) = last_net {
            let dt = at.duration_since(t0).as_secs_f64().max(0.1);
            s.net_down = rx.saturating_sub(r0) as f64 / dt;
            s.net_up = tx.saturating_sub(x0) as f64 / dt;
        }
        last_net = Some((at, (rx, tx)));
        if tick % 3 == 0 {
            ping = crate::net::ping();
        }
        if tick % 2 == 0 {
            procs = sampler.sample();
        }
        // Per-core loads, speed, memory details, and the drives (every 10 s).
        let now_cores = core_times();
        if now_cores.len() == last_cores.len() {
            s.cores = now_cores
                .iter()
                .zip(&last_cores)
                .map(|((i1, t1), (i0, t0))| {
                    let dt = t1.saturating_sub(*t0);
                    if dt == 0 {
                        0.0
                    } else {
                        (1.0 - i1.saturating_sub(*i0) as f64 / dt as f64).clamp(0.0, 1.0) as f32
                    }
                })
                .collect();
        }
        last_cores = now_cores;
        s.cpu_info = info.clone();
        s.mem_info = ram.clone();
        s.cpu_mhz = cpu_mhz(info.threads);
        s.uptime_secs = unsafe { windows::Win32::System::SystemInformation::GetTickCount64() } / 1000;
        s.perf = perf();
        if tick % 10 == 0 {
            drives_now = drives();
        }
        s.drives = drives_now.clone();
        if tick % 15 == 0 {
            batteries = crate::devices::batteries();
        }
        s.batteries = batteries.clone();
        s.procs = procs.clone();
        tick += 1;
        s.ping = ping;
        events::send(Event::Sys(s));
        std::thread::sleep(Duration::from_secs(1));
    }
}

// ---------- NVML (ships with the NVIDIA driver as System32\nvml.dll) ----------

#[repr(C)]
#[derive(Default)]
struct Utilization {
    gpu: u32,
    memory: u32,
}

#[repr(C)]
#[derive(Default)]
struct Memory {
    total: u64,
    free: u64,
    used: u64,
}

type Dev = *mut core::ffi::c_void;

pub(crate) struct Nvml {
    dev: Dev,
    name: String,
    util: unsafe extern "C" fn(Dev, *mut Utilization) -> i32,
    mem: unsafe extern "C" fn(Dev, *mut Memory) -> i32,
    temp: unsafe extern "C" fn(Dev, u32, *mut u32) -> i32,
    clock: Option<unsafe extern "C" fn(Dev, u32, *mut u32) -> i32>,
    power: Option<unsafe extern "C" fn(Dev, *mut u32) -> i32>,
    driver: String,
}

unsafe impl Send for Nvml {}

fn short_gpu_name(full: &str) -> String {
    full.replace("NVIDIA ", "").replace("GeForce ", "").replace(" Laptop GPU", "").trim().to_string()
}

impl Nvml {
    pub(crate) fn load() -> Option<Self> {
        unsafe {
            let lib: HMODULE = LoadLibraryW(w!("nvml.dll")).ok()?;
            let sym = |name: PCSTR| GetProcAddress(lib, name);
            let init: unsafe extern "C" fn() -> i32 = std::mem::transmute(sym(s!("nvmlInit_v2"))?);
            let by_index: unsafe extern "C" fn(u32, *mut Dev) -> i32 = std::mem::transmute(sym(s!("nvmlDeviceGetHandleByIndex_v2"))?);
            let get_name: unsafe extern "C" fn(Dev, *mut u8, u32) -> i32 = std::mem::transmute(sym(s!("nvmlDeviceGetName"))?);
            let util = std::mem::transmute(sym(s!("nvmlDeviceGetUtilizationRates"))?);
            let mem = std::mem::transmute(sym(s!("nvmlDeviceGetMemoryInfo"))?);
            let temp = std::mem::transmute(sym(s!("nvmlDeviceGetTemperature"))?);
            if init() != 0 {
                crate::util::log("NVML init failed; no GPU stats");
                return None;
            }
            let mut dev: Dev = std::ptr::null_mut();
            if by_index(0, &mut dev) != 0 {
                return None;
            }
            let mut buf = [0u8; 96];
            let name = if get_name(dev, buf.as_mut_ptr(), buf.len() as u32) == 0 {
                let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
                short_gpu_name(&String::from_utf8_lossy(&buf[..end]))
            } else {
                "GPU".into()
            };
            let clock = sym(s!("nvmlDeviceGetClockInfo")).map(|f| std::mem::transmute::<_, unsafe extern "C" fn(Dev, u32, *mut u32) -> i32>(f));
            let power = sym(s!("nvmlDeviceGetPowerUsage")).map(|f| std::mem::transmute::<_, unsafe extern "C" fn(Dev, *mut u32) -> i32>(f));
            let mut drv = [0u8; 96];
            let driver = match sym(s!("nvmlSystemGetDriverVersion")) {
                Some(f) => {
                    let f: unsafe extern "C" fn(*mut u8, u32) -> i32 = std::mem::transmute(f);
                    if f(drv.as_mut_ptr(), drv.len() as u32) == 0 {
                        let end = drv.iter().position(|&b| b == 0).unwrap_or(drv.len());
                        String::from_utf8_lossy(&drv[..end]).into_owned()
                    } else {
                        String::new()
                    }
                }
                None => String::new(),
            };
            Some(Self { dev, name, util, mem, temp, clock, power, driver })
        }
    }

    pub(crate) fn sample(&self) -> Option<Gpu> {
        unsafe {
            let mut u = Utilization::default();
            if (self.util)(self.dev, &mut u) != 0 {
                return None;
            }
            let mut m = Memory::default();
            let _ = (self.mem)(self.dev, &mut m);
            let mut t = 0u32;
            let temp = ((self.temp)(self.dev, 0, &mut t) == 0).then_some(t);
            let clock = |kind: u32| {
                let mut v = 0u32;
                self.clock.and_then(|f| (f(self.dev, kind, &mut v) == 0).then_some(v))
            };
            let mut mw = 0u32;
            let power = self.power.and_then(|f| (f(self.dev, &mut mw) == 0).then_some(mw as f32 / 1000.0));
            Some(Gpu {
                name: self.name.clone(),
                util: u.gpu as f32 / 100.0,
                vram_used: m.used,
                vram_total: m.total,
                temp,
                mem_util: u.memory as f32 / 100.0,
                clock: clock(0),
                mem_clock: clock(2),
                power,
                driver: self.driver.clone(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn gpu_names() {
        assert_eq!(super::short_gpu_name("NVIDIA GeForce RTX 5070 Ti Laptop GPU"), "RTX 5070 Ti");
    }
}

#[cfg(test)]
mod names_tests {
    use super::*;

    #[test]
    fn ram_and_disk_names() {
        // One type-17 entry: 32 GB DDR5, 5600 MT/s, maker "80CE" (Samsung), part "M425R4GA3BB0".
        let mut s = vec![0u8; 0x28];
        s[0] = 17;
        s[1] = 0x28;
        s[0x0C..0x0E].copy_from_slice(&0x7FFFu16.to_le_bytes());
        s[0x1C..0x20].copy_from_slice(&32768u32.to_le_bytes());
        s[0x12] = 0x22;
        s[0x15..0x17].copy_from_slice(&5600u16.to_le_bytes());
        s[0x17] = 1;
        s[0x1A] = 2;
        s.extend_from_slice(b"80CE000080CE\0M425R4GA3BB0   \0\0");
        let mut t = s.clone();
        t.extend_from_slice(&s);
        // An empty slot.
        let mut e = vec![0u8; 0x28];
        e[0] = 17;
        e[1] = 0x28;
        t.extend_from_slice(&e);
        t.extend_from_slice(&[0, 0]);
        t.extend_from_slice(&[127, 4, 0, 0, 0, 0]);
        let m = parse_smbios(&t);
        assert_eq!(m.slots, 3);
        assert_eq!(m.modules.len(), 2);
        assert_eq!(m.modules[0].part, "M425R4GA3BB0");
        assert_eq!(m.name(), "Samsung DDR5-5600");
        assert_eq!(m.sticks(), "2 × 32 GB");
        assert_eq!(jedec("Micron Technology"), "Micron");
        assert_eq!(jedec("Unknown"), "");
        // The real machine: something sensible or nothing, never a crash.
        let _ = mem_info();
        let _ = disk_model("C:");
    }
}
