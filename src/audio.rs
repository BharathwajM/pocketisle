//! Default output device volume: change notifications (for the pop-up) and control
//! (for the volume keys, which the island takes over from the Windows flyout).

use windows::core::{implement, Result, PCWSTR};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Media::Audio::Endpoints::*;
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL, STGM_READ};

use crate::events::{self, Event};

#[implement(IAudioEndpointVolumeCallback)]
struct VolumeCallback;

impl IAudioEndpointVolumeCallback_Impl for VolumeCallback_Impl {
    fn OnNotify(&self, data: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> Result<()> {
        if let Some(d) = unsafe { data.as_ref() } {
            events::send(Event::Volume { level: d.fMasterVolume, muted: d.bMuted.as_bool() });
        }
        Ok(())
    }
}

#[implement(IMMNotificationClient)]
struct DeviceCallback;

impl IMMNotificationClient_Impl for DeviceCallback_Impl {
    fn OnDeviceStateChanged(&self, _id: &PCWSTR, _state: DEVICE_STATE) -> Result<()> {
        Ok(())
    }
    fn OnDeviceAdded(&self, _id: &PCWSTR) -> Result<()> {
        Ok(())
    }
    fn OnDeviceRemoved(&self, _id: &PCWSTR) -> Result<()> {
        Ok(())
    }
    fn OnDefaultDeviceChanged(&self, flow: EDataFlow, role: ERole, _id: &PCWSTR) -> Result<()> {
        if flow == eRender && role == eConsole {
            events::send(Event::AudioDeviceChanged);
        }
        Ok(())
    }
    fn OnPropertyValueChanged(&self, _id: &PCWSTR, _key: &PROPERTYKEY) -> Result<()> {
        Ok(())
    }
}

pub struct Audio {
    enumerator: IMMDeviceEnumerator,
    endpoint: Option<IAudioEndpointVolume>,
    /// The output's peak level, for the island's tide and equaliser.
    meter: Option<IAudioMeterInformation>,
    volume_cb: IAudioEndpointVolumeCallback,
    device_cb: IMMNotificationClient,
    pub device: String,
}

impl Audio {
    pub fn new() -> Result<Self> {
        unsafe {
            let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let device_cb: IMMNotificationClient = DeviceCallback.into();
            enumerator.RegisterEndpointNotificationCallback(&device_cb)?;
            let mut a = Self {
                enumerator,
                endpoint: None,
                meter: None,
                volume_cb: VolumeCallback.into(),
                device_cb,
                device: String::new(),
            };
            a.bind();
            Ok(a)
        }
    }

    /// (Re)attach to the current default output device.
    pub fn bind(&mut self) {
        unsafe {
            if let Some(ep) = self.endpoint.take() {
                let _ = ep.UnregisterControlChangeNotify(&self.volume_cb);
            }
            let Ok(dev) = self.enumerator.GetDefaultAudioEndpoint(eRender, eConsole) else { return };
            self.device = dev
                .OpenPropertyStore(STGM_READ)
                .and_then(|s| s.GetValue(&PKEY_Device_FriendlyName))
                .map(|v| v.to_string())
                .unwrap_or_default();
            if let Ok(ep) = dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) {
                let _ = ep.RegisterControlChangeNotify(&self.volume_cb);
                self.endpoint = Some(ep);
            }
            self.meter = dev.Activate::<IAudioMeterInformation>(CLSCTX_ALL, None).ok();
        }
    }

    /// How loud the output is right now (0..1, before the volume slider). Cheap: a shared
    /// memory read, so the island polls it each frame while music animates.
    pub fn peak(&self) -> f32 {
        self.meter.as_ref().and_then(|m| unsafe { m.GetPeakValue().ok() }).unwrap_or(0.0).clamp(0.0, 1.0)
    }

    /// Change volume by `delta_pct` (unmuting on the way up). Returns the new (level, muted).
    pub fn step(&self, delta_pct: i32) -> Option<(f32, bool)> {
        let ep = self.endpoint.as_ref()?;
        unsafe {
            let cur = ep.GetMasterVolumeLevelScalar().unwrap_or(0.0);
            let pct = ((cur * 100.0).round() as i32 + delta_pct).clamp(0, 100);
            let mut muted = ep.GetMute().map(|m| m.as_bool()).unwrap_or(false);
            if delta_pct > 0 && muted {
                let _ = ep.SetMute(false, std::ptr::null());
                muted = false;
            }
            let level = pct as f32 / 100.0;
            let _ = ep.SetMasterVolumeLevelScalar(level, std::ptr::null());
            Some((level, muted))
        }
    }

    pub fn toggle_mute(&self) -> Option<(f32, bool)> {
        let ep = self.endpoint.as_ref()?;
        unsafe {
            let muted = !ep.GetMute().map(|m| m.as_bool()).unwrap_or(false);
            let _ = ep.SetMute(muted, std::ptr::null());
            Some((ep.GetMasterVolumeLevelScalar().unwrap_or(0.0), muted))
        }
    }

    pub fn level(&self) -> Option<(f32, bool)> {
        read(self.endpoint.as_ref()?)
    }

    /// Set the volume (0..1), unmuting when raised above zero.
    pub fn set_level(&self, v: f32) -> Option<(f32, bool)> {
        write(self.endpoint.as_ref()?, v)
    }

    /// The default microphone (looked up each time: it changes when headsets come and go).
    fn mic(&self) -> Option<IAudioEndpointVolume> {
        unsafe {
            let dev = self.enumerator.GetDefaultAudioEndpoint(eCapture, eConsole).ok()?;
            dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok()
        }
    }

    pub fn mic_level(&self) -> Option<(f32, bool)> {
        read(&self.mic()?)
    }

    pub fn set_mic_level(&self, v: f32) -> Option<(f32, bool)> {
        write(&self.mic()?, v)
    }

    pub fn toggle_mic_mute(&self) -> Option<(f32, bool)> {
        let ep = self.mic()?;
        unsafe {
            let muted = !ep.GetMute().map(|m| m.as_bool()).unwrap_or(false);
            let _ = ep.SetMute(muted, std::ptr::null());
        }
        read(&ep)
    }
}

fn read(ep: &IAudioEndpointVolume) -> Option<(f32, bool)> {
    unsafe { Some((ep.GetMasterVolumeLevelScalar().ok()?, ep.GetMute().map(|m| m.as_bool()).unwrap_or(false))) }
}

fn write(ep: &IAudioEndpointVolume, v: f32) -> Option<(f32, bool)> {
    let v = v.clamp(0.0, 1.0);
    unsafe {
        let _ = ep.SetMasterVolumeLevelScalar(v, std::ptr::null());
        if v > 0.0 && ep.GetMute().map(|m| m.as_bool()).unwrap_or(false) {
            let _ = ep.SetMute(false, std::ptr::null());
        }
    }
    read(ep)
}

impl Drop for Audio {
    fn drop(&mut self) {
        unsafe {
            if let Some(ep) = self.endpoint.take() {
                let _ = ep.UnregisterControlChangeNotify(&self.volume_cb);
            }
            let _ = self.enumerator.UnregisterEndpointNotificationCallback(&self.device_cb);
        }
    }
}
