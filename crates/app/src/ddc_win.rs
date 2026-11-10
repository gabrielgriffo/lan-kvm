use std::collections::HashMap;
use std::mem;
use std::ptr;
use std::thread::sleep;
use std::time::Duration;

use lan_kvm_core::ddc::{input_values, same_monitor, VCP_INPUT};
use lan_kvm_core::switch::Rect;
use windows_sys::Win32::Devices::Display::{
    CapabilitiesRequestAndCapabilitiesReply, DestroyPhysicalMonitor, DisplayConfigGetDeviceInfo,
    GetCapabilitiesStringLength, GetDisplayConfigBufferSizes,
    GetNumberOfPhysicalMonitorsFromHMONITOR, GetPhysicalMonitorsFromHMONITOR,
    GetVCPFeatureAndVCPFeatureReply, QueryDisplayConfig, SetVCPFeature,
    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
    DISPLAYCONFIG_DEVICE_INFO_HEADER, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO,
    DISPLAYCONFIG_SOURCE_DEVICE_NAME, DISPLAYCONFIG_TARGET_DEVICE_NAME, PHYSICAL_MONITOR,
    QDC_ONLY_ACTIVE_PATHS,
};
use windows_sys::Win32::Foundation::{GetLastError, HANDLE, LPARAM, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
};

const RETRY_DELAY: Duration = Duration::from_millis(40);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Monitor {
    pub name: String,
    pub inputs: Vec<u8>,
    pub current: Option<u8>,
}

struct Physical {
    handle: HANDLE,
    name: String,
}

impl Drop for Physical {
    fn drop(&mut self) {
        unsafe { DestroyPhysicalMonitor(self.handle) };
    }
}

fn wide(buffer: &[u16]) -> String {
    let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end]).trim().to_string()
}

fn friendly_names() -> HashMap<String, Vec<String>> {
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    unsafe {
        let (mut paths_len, mut modes_len) = (0u32, 0u32);
        if GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut paths_len, &mut modes_len) != 0 {
            return map;
        }
        let mut paths: Vec<DISPLAYCONFIG_PATH_INFO> = vec![mem::zeroed(); paths_len as usize];
        let mut modes: Vec<DISPLAYCONFIG_MODE_INFO> = vec![mem::zeroed(); modes_len as usize];
        if QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &mut paths_len,
            paths.as_mut_ptr(),
            &mut modes_len,
            modes.as_mut_ptr(),
            ptr::null_mut(),
        ) != 0
        {
            return map;
        }
        for path in &paths[..paths_len as usize] {
            let mut source: DISPLAYCONFIG_SOURCE_DEVICE_NAME = mem::zeroed();
            source.header = DISPLAYCONFIG_DEVICE_INFO_HEADER {
                r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
                size: mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32,
                adapterId: path.sourceInfo.adapterId,
                id: path.sourceInfo.id,
            };
            if DisplayConfigGetDeviceInfo(&mut source.header) != 0 {
                continue;
            }
            let mut target: DISPLAYCONFIG_TARGET_DEVICE_NAME = mem::zeroed();
            target.header = DISPLAYCONFIG_DEVICE_INFO_HEADER {
                r#type: DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
                size: mem::size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32,
                adapterId: path.targetInfo.adapterId,
                id: path.targetInfo.id,
            };
            let name = if DisplayConfigGetDeviceInfo(&mut target.header) == 0 {
                wide(&target.monitorFriendlyDeviceName)
            } else {
                String::new()
            };
            map.entry(wide(&source.viewGdiDeviceName))
                .or_default()
                .push(name);
        }
    }
    map
}

fn logical_monitors() -> Vec<HMONITOR> {
    unsafe extern "system" fn collect(m: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> i32 {
        (*(data as *mut Vec<HMONITOR>)).push(m);
        1
    }
    let mut logical: Vec<HMONITOR> = Vec::new();
    unsafe {
        EnumDisplayMonitors(
            ptr::null_mut(),
            ptr::null(),
            Some(collect),
            &mut logical as *mut _ as LPARAM,
        )
    };
    logical
}

pub fn screen_of(name: &str) -> Option<Rect> {
    let names = friendly_names();
    logical_monitors().into_iter().find_map(|hmonitor| unsafe {
        let mut info: MONITORINFOEXW = mem::zeroed();
        info.monitorInfo.cbSize = mem::size_of::<MONITORINFOEXW>() as u32;
        if GetMonitorInfoW(hmonitor, &mut info as *mut _ as *mut MONITORINFO) == 0 {
            return None;
        }
        let ours = names
            .get(&wide(&info.szDevice))
            .is_some_and(|list| list.iter().any(|n| same_monitor(n, name)));
        let r = info.monitorInfo.rcMonitor;
        ours.then_some(Rect {
            x: r.left,
            y: r.top,
            w: r.right - r.left,
            h: r.bottom - r.top,
        })
    })
}

fn physical_monitors() -> Vec<Physical> {
    let names = friendly_names();
    let mut out = Vec::new();
    for hmonitor in logical_monitors() {
        unsafe {
            let mut info: MONITORINFOEXW = mem::zeroed();
            info.monitorInfo.cbSize = mem::size_of::<MONITORINFOEXW>() as u32;
            if GetMonitorInfoW(hmonitor, &mut info as *mut _ as *mut MONITORINFO) == 0 {
                continue;
            }
            let device = wide(&info.szDevice);
            let mut count = 0u32;
            if GetNumberOfPhysicalMonitorsFromHMONITOR(hmonitor, &mut count) == 0 || count == 0 {
                continue;
            }
            let mut list: Vec<PHYSICAL_MONITOR> = vec![mem::zeroed(); count as usize];
            if GetPhysicalMonitorsFromHMONITOR(hmonitor, count, list.as_mut_ptr()) == 0 {
                continue;
            }
            for (i, physical) in list.iter().enumerate() {
                let friendly = names
                    .get(&device)
                    .and_then(|v| v.get(i))
                    .filter(|n| !n.is_empty())
                    .cloned();
                let (handle, text) = (
                    physical.hPhysicalMonitor,
                    physical.szPhysicalMonitorDescription,
                );
                let description = wide(&text);
                out.push(Physical {
                    handle,
                    name: friendly.unwrap_or(description),
                });
            }
        }
    }
    out
}

fn capabilities(handle: HANDLE) -> Option<String> {
    for attempt in 0..3 {
        if attempt > 0 {
            sleep(RETRY_DELAY * 3);
        }
        let mut length = 0u32;
        if unsafe { GetCapabilitiesStringLength(handle, &mut length) } == 0 || length == 0 {
            continue;
        }
        let mut buffer = vec![0u8; length as usize];
        if unsafe { CapabilitiesRequestAndCapabilitiesReply(handle, buffer.as_mut_ptr(), length) }
            == 0
        {
            continue;
        }
        let end = buffer.iter().position(|&b| b == 0).unwrap_or(buffer.len());
        return Some(String::from_utf8_lossy(&buffer[..end]).into_owned());
    }
    None
}

fn current_input(handle: HANDLE) -> Option<u8> {
    for attempt in 0..3 {
        if attempt > 0 {
            sleep(RETRY_DELAY);
        }
        let (mut current, mut maximum) = (0u32, 0u32);
        let ok = unsafe {
            GetVCPFeatureAndVCPFeatureReply(
                handle,
                VCP_INPUT,
                ptr::null_mut(),
                &mut current,
                &mut maximum,
            )
        };
        if ok != 0 {
            return Some((current & 0xFF) as u8);
        }
    }
    None
}

pub fn list() -> Vec<Monitor> {
    physical_monitors()
        .iter()
        .map(|m| Monitor {
            name: m.name.clone(),
            inputs: capabilities(m.handle)
                .map(|c| input_values(&c))
                .unwrap_or_default(),
            current: current_input(m.handle),
        })
        .collect()
}

pub fn read_input(name: &str) -> Option<u8> {
    physical_monitors()
        .iter()
        .find(|m| same_monitor(&m.name, name))
        .and_then(|m| current_input(m.handle))
}

pub fn set_input(name: &str, value: u8) -> Result<(), String> {
    let monitors = physical_monitors();
    let monitor = monitors
        .iter()
        .find(|m| same_monitor(&m.name, name))
        .ok_or_else(|| format!("monitor \"{name}\" não encontrado neste PC"))?;
    let mut code = 0;
    for attempt in 0..3 {
        if attempt > 0 {
            sleep(RETRY_DELAY);
        }
        if unsafe { SetVCPFeature(monitor.handle, VCP_INPUT, value as u32) } != 0 {
            return Ok(());
        }
        code = unsafe { GetLastError() };
    }
    Err(format!(
        "o monitor \"{}\" recusou a troca de entrada (erro 0x{code:08X})",
        monitor.name
    ))
}
