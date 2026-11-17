//! Laptop panel brightness changes, pushed by WMI (`WmiMonitorBrightnessEvent`).
//! The thread blocks inside WMI between events, so it costs nothing while idle.
//! On a desktop with no internal panel the query fails and the thread just exits.

use windows::core::{w, Result, BSTR};
use windows::Win32::System::Com::*;
use windows::Win32::System::Variant::{VariantToInt32, VARIANT};
use windows::Win32::System::Wmi::*;

use crate::events::{self, Event};

pub fn spawn() {
    std::thread::spawn(|| {
        if let Err(e) = run() {
            crate::util::log(&format!("brightness watcher stopped: {e}"));
        }
    });
}

/// Brightness changes from the Controls slider. Each request replaces any still waiting, so
/// dragging doesn't queue up dozens of (slow) WMI calls. Reports the current level first.
pub fn spawn_setter() -> std::sync::mpsc::Sender<u8> {
    let (tx, rx) = std::sync::mpsc::channel::<u8>();
    std::thread::spawn(move || {
        let r: Result<()> = (|| unsafe {
            let services = connect()?;
            // Current level, for the slider.
            let q = services.ExecQuery(
                &BSTR::from("WQL"),
                &BSTR::from("SELECT CurrentBrightness FROM WmiMonitorBrightness"),
                WBEM_FLAG_RETURN_IMMEDIATELY | WBEM_FLAG_FORWARD_ONLY,
                None,
            )?;
            let mut objs = [None];
            let mut n = 0u32;
            if q.Next(WBEM_INFINITE, &mut objs, &mut n).is_ok() && n == 1 {
                if let Some(o) = &objs[0] {
                    let mut v = VARIANT::default();
                    if o.Get(w!("CurrentBrightness"), 0, &mut v, None, None).is_ok() {
                        events::send(Event::BrightnessLevel(VariantToInt32(&v).unwrap_or(0).clamp(0, 100) as u8));
                    }
                }
            }
            // The method object's path and its input parameters.
            let q = services.ExecQuery(
                &BSTR::from("WQL"),
                &BSTR::from("SELECT * FROM WmiMonitorBrightnessMethods"),
                WBEM_FLAG_RETURN_IMMEDIATELY | WBEM_FLAG_FORWARD_ONLY,
                None,
            )?;
            let mut objs = [None];
            let mut n = 0u32;
            q.Next(WBEM_INFINITE, &mut objs, &mut n).ok()?;
            let Some(inst) = objs[0].take() else { return Ok(()) };
            let mut path = VARIANT::default();
            inst.Get(w!("__PATH"), 0, &mut path, None, None)?;
            let path = BSTR::from(path.to_string());
            let mut class = None;
            services.GetObject(&BSTR::from("WmiMonitorBrightnessMethods"), WBEM_GENERIC_FLAG_TYPE(0), None, Some(&mut class), None)?;
            let Some(class) = class else { return Ok(()) };
            let mut in_def = None;
            class.GetMethod(w!("WmiSetBrightness"), 0, &mut in_def, std::ptr::null_mut())?;
            let Some(in_def) = in_def else { return Ok(()) };
            while let Ok(mut level) = rx.recv() {
                while let Ok(newer) = rx.try_recv() {
                    level = newer;
                }
                let args = in_def.SpawnInstance(0)?;
                args.Put(w!("Timeout"), 0, &VARIANT::from(0i32), 0)?;
                args.Put(w!("Brightness"), 0, &VARIANT::from(level.min(100)), 0)?;
                if let Err(e) = services.ExecMethod(&path, &BSTR::from("WmiSetBrightness"), WBEM_GENERIC_FLAG_TYPE(0), None, &args, None, None) {
                    crate::util::log(&format!("set brightness failed: {e}"));
                }
            }
            Ok(())
        })();
        if let Err(e) = r {
            crate::util::log(&format!("brightness control unavailable: {e}"));
        }
    });
    tx
}

unsafe fn connect() -> Result<IWbemServices> {
    CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
    let locator: IWbemLocator = CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER)?;
    let services = locator.ConnectServer(&BSTR::from("ROOT\\WMI"), &BSTR::new(), &BSTR::new(), &BSTR::new(), 0, &BSTR::new(), None)?;
    CoSetProxyBlanket(&services, 10, 0, None, RPC_C_AUTHN_LEVEL_CALL, RPC_C_IMP_LEVEL_IMPERSONATE, None, EOAC_NONE)?;
    Ok(services)
}

fn run() -> Result<()> {
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        let locator: IWbemLocator = CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER)?;
        let services = locator.ConnectServer(
            &BSTR::from("ROOT\\WMI"),
            &BSTR::new(),
            &BSTR::new(),
            &BSTR::new(),
            0,
            &BSTR::new(),
            None,
        )?;
        CoSetProxyBlanket(
            &services,
            10, // RPC_C_AUTHN_WINNT
            0,  // RPC_C_AUTHZ_NONE
            None,
            RPC_C_AUTHN_LEVEL_CALL,
            RPC_C_IMP_LEVEL_IMPERSONATE,
            None,
            EOAC_NONE,
        )?;
        let events = services.ExecNotificationQuery(
            &BSTR::from("WQL"),
            &BSTR::from("SELECT * FROM WmiMonitorBrightnessEvent"),
            WBEM_FLAG_RETURN_IMMEDIATELY | WBEM_FLAG_FORWARD_ONLY,
            None,
        )?;
        loop {
            let mut objs = [None];
            let mut returned = 0u32;
            events.Next(WBEM_INFINITE, &mut objs, &mut returned).ok()?;
            if returned == 0 {
                continue;
            }
            if let Some(obj) = &objs[0] {
                let mut v = VARIANT::default();
                if obj.Get(w!("Brightness"), 0, &mut v, None, None).is_ok() {
                    let b = VariantToInt32(&v).unwrap_or(0).clamp(0, 100) as u8;
                    events::send(Event::Brightness(b));
                }
            }
        }
    }
}
