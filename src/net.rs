//! The network: whether Wi-Fi / Ethernet / a VPN is up (looked at every 2 s, reported when it
//! has changed and stayed changed), plus byte counters and a ping for the System tab.

use std::time::Duration;

use windows::core::GUID;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::NetworkManagement::IpHelper::*;
use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;
use windows::Win32::NetworkManagement::WiFi::*;

use crate::events::{self, Event};

#[derive(Clone, Default, PartialEq, Debug)]
pub struct Net {
    /// Connected to Wi-Fi: the network's name ("" when Windows won't say).
    pub wifi: Option<String>,
    pub wired: bool,
    /// A VPN is up: its name.
    pub vpn: Option<String>,
}

impl Net {
    pub fn online(&self) -> bool {
        self.wifi.is_some() || self.wired
    }
}

const IF_ETHERNET: u32 = 6;
const IF_PPP: u32 = 23;
const IF_WIFI: u32 = 71;

const VPN_WORDS: [&str; 20] = [
    "vpn", "wireguard", "openvpn", "tap-windows", "wintun", "tailscale", "nordlynx", "zerotier", "warp", "fortinet",
    "anyconnect", "globalprotect", "pangp", "proton", "expressvpn", "mullvad", "surfshark", "windscribe", "pulse secure", "sonicwall",
];

fn wide(s: &[u16]) -> String {
    let n = s.iter().position(|&c| c == 0).unwrap_or(s.len());
    String::from_utf16_lossy(&s[..n])
}

/// Every interface row, handed to `f`.
fn each_if(mut f: impl FnMut(&MIB_IF_ROW2)) {
    unsafe {
        let mut t: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
        if GetIfTable2(&mut t).is_err() || t.is_null() {
            return;
        }
        let n = (*t).NumEntries as usize;
        let rows = std::slice::from_raw_parts((*t).Table.as_ptr(), n);
        for r in rows {
            f(r);
        }
        FreeMibTable(t as *const _);
    }
}

fn hardware(r: &MIB_IF_ROW2) -> bool {
    let b = r.InterfaceAndOperStatusFlags._bitfield;
    b & 1 != 0 && b & 2 == 0 // HardwareInterface, not a FilterInterface
}

/// Bytes received and sent so far over the real adapters.
pub fn totals() -> (u64, u64) {
    let (mut rx, mut tx) = (0u64, 0u64);
    each_if(|r| {
        if hardware(r) && r.OperStatus == IfOperStatusUp {
            rx += r.InOctets;
            tx += r.OutOctets;
        }
    });
    (rx, tx)
}

/// Round trip to 1.1.1.1 in ms.
pub fn ping() -> Option<u32> {
    unsafe {
        let h = IcmpCreateFile().ok()?;
        let data = [0u8; 8];
        let mut reply = [0u8; 128];
        let dest = u32::from_ne_bytes([1, 1, 1, 1]);
        let n = IcmpSendEcho(h, dest, data.as_ptr() as *const _, data.len() as u16, None, reply.as_mut_ptr() as *mut _, reply.len() as u32, 900);
        let _ = IcmpCloseHandle(h);
        if n == 0 {
            return None;
        }
        let r = &*(reply.as_ptr() as *const ICMP_ECHO_REPLY);
        (r.Status == 0).then_some(r.RoundTripTime)
    }
}

fn ssid(wlan: HANDLE) -> Option<String> {
    unsafe {
        let mut list: *mut WLAN_INTERFACE_INFO_LIST = std::ptr::null_mut();
        if WlanEnumInterfaces(wlan, None, &mut list) != 0 || list.is_null() {
            return None;
        }
        let n = (*list).dwNumberOfItems as usize;
        let items = std::slice::from_raw_parts((*list).InterfaceInfo.as_ptr(), n);
        let mut out = None;
        for i in items {
            if i.isState != wlan_interface_state_connected {
                continue;
            }
            let guid: GUID = i.InterfaceGuid;
            let mut size = 0u32;
            let mut data: *mut core::ffi::c_void = std::ptr::null_mut();
            if WlanQueryInterface(wlan, &guid, wlan_intf_opcode_current_connection, None, &mut size, &mut data, None) == 0 && !data.is_null() {
                let a = &*(data as *const WLAN_CONNECTION_ATTRIBUTES);
                let s = &a.wlanAssociationAttributes.dot11Ssid;
                let len = (s.uSSIDLength as usize).min(32);
                out = Some(String::from_utf8_lossy(&s.ucSSID[..len]).into_owned());
                WlanFreeMemory(data);
                break;
            }
        }
        WlanFreeMemory(list as *const _);
        out
    }
}

fn read(wlan: Option<HANDLE>) -> Net {
    let mut n = Net::default();
    let mut wifi = false;
    each_if(|r| {
        if r.OperStatus != IfOperStatusUp {
            return;
        }
        let desc = wide(&r.Description).to_lowercase();
        let alias = wide(&r.Alias);
        if hardware(r) && r.Type == IF_WIFI {
            wifi = true;
        } else if hardware(r) && r.Type == IF_ETHERNET && !VPN_WORDS.iter().any(|w| desc.contains(w)) {
            n.wired = true;
        } else if r.InterfaceAndOperStatusFlags._bitfield & 2 == 0 {
            let al = alias.to_lowercase();
            if (r.Type == IF_PPP && !desc.contains("miniport")) || VPN_WORDS.iter().any(|w| desc.contains(w) || al.contains(w)) {
                n.vpn.get_or_insert(alias);
            }
        }
    });
    if wifi {
        n.wifi = Some(wlan.and_then(ssid).unwrap_or_default());
    }
    n
}

pub fn spawn() {
    let _ = std::thread::Builder::new().name("net".into()).spawn(|| {
        let wlan = unsafe {
            let mut ver = 0u32;
            let mut h = HANDLE::default();
            (WlanOpenHandle(2, None, &mut ver, &mut h) == 0).then_some(h)
        };
        let mut sent: Option<Net> = None;
        let mut seen: Option<Net> = None;
        loop {
            let now = read(wlan);
            // Report a change once it has held for two looks (a blip isn't news).
            if sent.as_ref() != Some(&now) && (sent.is_none() || seen.as_ref() == Some(&now)) {
                events::send(Event::Net(now.clone()));
                sent = Some(now.clone());
            }
            seen = Some(now);
            std::thread::sleep(Duration::from_secs(2));
        }
    });
}
