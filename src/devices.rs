//! Devices: the battery levels Windows knows for Bluetooth devices (earbuds, mice,
//! keyboards, controllers), and the sound outputs, with a way to switch the default one.

use windows::core::{Interface, GUID, PCWSTR};
use windows::Win32::Devices::DeviceAndDriverInstallation::*;
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Devices::Properties::{DEVPKEY_Device_ClassGuid, DEVPKEY_Device_ContainerId, DEVPKEY_NAME, DEVPROPTYPE};
use windows::Win32::Foundation::{DEVPROPKEY, PROPERTYKEY};
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_ALL, STGM_READ};

use crate::peeks::{guid_from, normalise_guid};

/// A Bluetooth device and its battery (0..100).
#[derive(Clone, Debug, PartialEq)]
pub struct Battery {
    pub name: String,
    pub pct: u8,
    pub kind: DevKind,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DevKind {
    Audio,
    Mouse,
    Keyboard,
    Controller,
    Pen,
    Phone,
    Other,
}

const CLASS_BLUETOOTH: u128 = 0xe0cbf06c_cd8b_4647_bb8a_263b43f0f974;
const CLASS_MOUSE: u128 = 0x4d36e96f_e325_11ce_bfc1_08002be10318;
const CLASS_KEYBOARD: u128 = 0x4d36e96b_e325_11ce_bfc1_08002be10318;
const CLASS_MEDIA: u128 = 0x4d36e96c_e325_11ce_bfc1_08002be10318;
const CLASS_AUDIO_ENDPOINT: u128 = 0xc166523c_fe0c_4a94_a586_f1a80cfbbf3e;
const CLASS_XBOX: u128 = 0x05f5cfe2_4733_4950_a6bb_07aad01a3a84;
/// The computer's own container: everything built in.
const THIS_PC: &str = "00000000-0000-0000-FFFF-FFFFFFFFFFFF";

/// DEVPKEY_Bluetooth_Battery: what Settings > Bluetooth & devices shows.
const BATTERY: DEVPROPKEY = DEVPROPKEY { fmtid: GUID::from_u128(0x104ea319_6ee2_4701_bd47_8ddbf425bbe5), pid: 2 };
/// DEVPKEY_Device_IsConnected (Bluetooth): false while it's paired but off or away.
const CONNECTED: DEVPROPKEY = DEVPROPKEY { fmtid: GUID::from_u128(0x83da6326_97a6_4088_9453_a1923f573b29), pid: 15 };

#[derive(Default)]
struct Box_ {
    name: String,
    bt_name: String,
    /// The name of the node that carries the battery (the device itself).
    pct_name: String,
    pct: Option<u8>,
    connected: Option<bool>,
    classes: Vec<u128>,
}

/// Every Bluetooth device Windows reports a battery for, connected ones only.
pub fn batteries() -> Vec<Battery> {
    let mut by: Vec<(String, Box_)> = Vec::new();
    unsafe {
        let Ok(set) = SetupDiGetClassDevsW(None, PCWSTR::null(), None, DIGCF_ALLCLASSES | DIGCF_PRESENT) else { return Vec::new() };
        let mut i = 0;
        loop {
            let mut d = SP_DEVINFO_DATA { cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32, ..Default::default() };
            if SetupDiEnumDeviceInfo(set, i, &mut d).is_err() {
                break;
            }
            i += 1;
            let mut ty = DEVPROPTYPE::default();
            let mut g = [0u8; 16];
            if SetupDiGetDevicePropertyW(set, &d, &DEVPKEY_Device_ContainerId, &mut ty, Some(&mut g), None, 0).is_err() {
                continue;
            }
            let container = normalise_guid(&format!("{:?}", guid_from(&g)));
            if container == THIS_PC {
                continue;
            }
            let mut c = [0u8; 16];
            let class = if SetupDiGetDevicePropertyW(set, &d, &DEVPKEY_Device_ClassGuid, &mut ty, Some(&mut c), None, 0).is_ok() { guid_from(&c).to_u128() } else { 0 };
            let name = string_prop(set, &d, &DEVPKEY_NAME);
            let mut b = [0u8; 1];
            let pct = (SetupDiGetDevicePropertyW(set, &d, &BATTERY, &mut ty, Some(&mut b), None, 0).is_ok() && b[0] <= 100).then_some(b[0]);
            let mut on = [0u8; 1];
            let connected = SetupDiGetDevicePropertyW(set, &d, &CONNECTED, &mut ty, Some(&mut on), None, 0).is_ok().then_some(on[0] != 0);
            let slot = match by.iter().position(|(k, _)| *k == container) {
                Some(j) => j,
                None => {
                    by.push((container, Box_::default()));
                    by.len() - 1
                }
            };
            let e = &mut by[slot].1;
            let real = !name.is_empty() && !generic(&name);
            if class == CLASS_BLUETOOTH && real && e.bt_name.is_empty() {
                e.bt_name = name.clone();
            }
            if pct.is_some() {
                e.pct = pct;
                if real && e.pct_name.is_empty() {
                    e.pct_name = name.clone();
                }
            }
            if real && (e.name.is_empty() || name.len() < e.name.len()) {
                e.name = name;
            }
            if let Some(c) = connected {
                e.connected = Some(e.connected.unwrap_or(false) || c);
            }
            if !e.classes.contains(&class) {
                e.classes.push(class);
            }
        }
        let _ = SetupDiDestroyDeviceInfoList(set);
    }
    let mut out: Vec<Battery> = by
        .into_iter()
        .filter_map(|(_, e)| {
            let pct = e.pct?;
            if e.connected == Some(false) {
                return None;
            }
            let name = [e.pct_name, e.bt_name, e.name].into_iter().find(|n| !n.is_empty())?;
            let kind = kind_of(&e.classes, &name);
            Some(Battery { name: tidy(&name), pct, kind })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out.dedup_by(|a, b| a.name == b.name);
    out
}

fn kind_of(classes: &[u128], name: &str) -> DevKind {
    let n = name.to_lowercase();
    if classes.contains(&CLASS_XBOX) || n.contains("controller") || n.contains("gamepad") || n.contains("dualsense") || n.contains("joy-con") {
        DevKind::Controller
    } else if ["iphone", "ipad", "pixel", "galaxy s", "galaxy z", "galaxy a", "oneplus"].iter().any(|k| n.contains(k)) {
        DevKind::Phone
    } else if n.contains("pen") || n.contains("stylus") {
        DevKind::Pen
    } else if classes.contains(&CLASS_AUDIO_ENDPOINT) || classes.contains(&CLASS_MEDIA) || ["buds", "airpods", "headphone", "headset", "wh-", "wf-", "earbud"].iter().any(|k| n.contains(k)) {
        DevKind::Audio
    } else if classes.contains(&CLASS_MOUSE) || n.contains("mouse") || n.contains("mx ") {
        DevKind::Mouse
    } else if classes.contains(&CLASS_KEYBOARD) || n.contains("keyboard") || n.contains("keys") {
        DevKind::Keyboard
    } else {
        DevKind::Other
    }
}

/// The names Windows gives a device's Bluetooth services, not the device: "Service
/// Discovery Service", "Generic Attribute Profile", "Avrcp Transport"…
fn generic(name: &str) -> bool {
    let n = name.trim().to_lowercase();
    n.ends_with(" service") || n.ends_with(" profile") || n.ends_with(" transport") || n.starts_with("bluetooth ") || n.starts_with("microsoft bluetooth") || n.starts_with("generic ")
}

/// "WH-1000XM4 Hands-Free AG" → "WH-1000XM4".
fn tidy(name: &str) -> String {
    let mut s = name.trim().to_string();
    for tail in [" Hands-Free AG Audio", " Hands-Free AG", " Hands-Free", " Avrcp Transport", " Stereo", " LE"] {
        if let Some(t) = s.strip_suffix(tail) {
            s = t.trim().to_string();
        }
    }
    s
}

unsafe fn string_prop(set: HDEVINFO, d: &SP_DEVINFO_DATA, key: &DEVPROPKEY) -> String {
    let mut ty = DEVPROPTYPE::default();
    let mut buf = vec![0u8; 512];
    let mut need = 0u32;
    if SetupDiGetDevicePropertyW(set, d, key, &mut ty, Some(&mut buf), Some(&mut need), 0).is_err() {
        return String::new();
    }
    let units: Vec<u16> = buf[..(need as usize).min(buf.len())].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|u| *u != 0).collect();
    String::from_utf16_lossy(&units)
}

/// A sound output.
#[derive(Clone, Debug, PartialEq)]
pub struct Output {
    pub id: String,
    /// "Speakers", "Headphones", "LG ULTRAGEAR"…
    pub name: String,
    /// The hardware behind it: "Realtek(R) Audio", "WH-1000XM4"…
    pub hw: String,
    /// EndpointFormFactor: 1 speakers, 3 headphones, 5 headset, 9 a monitor or TV.
    pub form: u32,
    pub default: bool,
}

const PKEY_DEVICE_DESC: PROPERTYKEY = PROPERTYKEY { fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0), pid: 2 };
const PKEY_INTERFACE_NAME: PROPERTYKEY = PROPERTYKEY { fmtid: GUID::from_u128(0x026e516e_b814_414b_83cd_856d6fef4822), pid: 2 };

/// The sound outputs that are plugged in and on, the default one marked.
pub fn outputs() -> Vec<Output> {
    let mut out = Vec::new();
    unsafe {
        let Ok(en) = CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL) else { return out };
        let default = en.GetDefaultAudioEndpoint(eRender, eConsole).ok().and_then(|d| id_of(&d)).unwrap_or_default();
        let Ok(list) = en.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) else { return out };
        let n = list.GetCount().unwrap_or(0);
        for i in 0..n {
            let Ok(dev) = list.Item(i) else { continue };
            let Some(id) = id_of(&dev) else { continue };
            let Ok(store) = dev.OpenPropertyStore(STGM_READ) else { continue };
            let get = |k: &PROPERTYKEY| store.GetValue(k).map(|v| v.to_string()).unwrap_or_default();
            let mut name = get(&PKEY_DEVICE_DESC);
            let hw = get(&PKEY_INTERFACE_NAME);
            if name.is_empty() {
                name = get(&PKEY_Device_FriendlyName);
            }
            let form = store.GetValue(&PKEY_AudioEndpoint_FormFactor).ok().and_then(|v| u32::try_from(&v).ok()).unwrap_or(10);
            out.push(Output { default: id == default, id, name, hw, form });
        }
    }
    out
}

unsafe fn id_of(dev: &IMMDevice) -> Option<String> {
    let p = dev.GetId().ok()?;
    let s = p.to_string().ok();
    CoTaskMemFree(Some(p.0 as _));
    s
}

/// Make this the default output (for every role, like the Windows sound menu does). Uses
/// the policy interface the Windows sound settings use; nothing else changes.
pub fn set_default_output(id: &str) -> bool {
    const CLSID_POLICY_CONFIG: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);
    const IID_POLICY_CONFIG: GUID = GUID::from_u128(0xf8679f50_850a_41cf_9c72_430f290290c8);
    let wide: Vec<u16> = id.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let Ok(unk) = CoCreateInstance::<_, windows::core::IUnknown>(&CLSID_POLICY_CONFIG, None, CLSCTX_ALL) else { return false };
        let mut p: *mut std::ffi::c_void = std::ptr::null_mut();
        if unk.query(&IID_POLICY_CONFIG, &mut p).is_err() || p.is_null() {
            return false;
        }
        // IPolicyConfig: IUnknown's three, then GetMixFormat, GetDeviceFormat,
        // ResetDeviceFormat, SetDeviceFormat, GetProcessingPeriod, SetProcessingPeriod,
        // GetShareMode, SetShareMode, GetPropertyValue, SetPropertyValue, SetDefaultEndpoint.
        let vtbl = *(p as *const *const usize);
        let set: extern "system" fn(*mut std::ffi::c_void, PCWSTR, i32) -> windows::core::HRESULT = std::mem::transmute(*vtbl.add(13));
        let release: extern "system" fn(*mut std::ffi::c_void) -> u32 = std::mem::transmute(*vtbl.add(2));
        let mut ok = true;
        for role in [eConsole, eMultimedia, eCommunications] {
            ok &= set(p, PCWSTR(wide.as_ptr()), role.0).is_ok();
        }
        release(p);
        ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_tidy_up() {
        assert_eq!(tidy("WH-1000XM4 Hands-Free AG"), "WH-1000XM4");
        assert_eq!(tidy("MX Master 3"), "MX Master 3");
        assert_eq!(kind_of(&[], "Xbox Wireless Controller"), DevKind::Controller);
        assert_eq!(kind_of(&[CLASS_MOUSE], "MX Master 3"), DevKind::Mouse);
        assert_eq!(kind_of(&[], "Galaxy Buds2 Pro"), DevKind::Audio);
        assert_eq!(kind_of(&[CLASS_MEDIA], "My iPhone"), DevKind::Phone);
        assert!(generic("Service Discovery Service"));
        assert!(generic("Generic Attribute Profile"));
        assert!(!generic("WH-1000XM4"));
    }

    /// What this PC has (run by hand: cargo test devices_here -- --ignored --nocapture).
    #[test]
    #[ignore]
    fn devices_here() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        for b in batteries() {
            println!("battery: {} {}% {:?}", b.name, b.pct, b.kind);
        }
        for o in outputs() {
            println!("output: {} | {} | form {} | default {}", o.name, o.hw, o.form, o.default);
        }
    }
}
