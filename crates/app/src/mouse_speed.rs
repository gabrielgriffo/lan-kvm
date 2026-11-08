use std::ptr;

use lan_kvm_core::ballistics::{parse_curve, Ballistics, DEFAULT_CURVE_X, DEFAULT_CURVE_Y};
use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_BINARY};
use windows_sys::Win32::UI::HiDpi::GetDpiForSystem;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    SystemParametersInfoW, SPI_GETMOUSE, SPI_GETMOUSESPEED,
};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn curve(name: &str) -> Option<[f64; 5]> {
    let key = wide(r"Control Panel\Mouse");
    let value = wide(name);
    let mut buf = [0u8; 64];
    let mut len = buf.len() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_BINARY,
            ptr::null_mut(),
            buf.as_mut_ptr().cast(),
            &mut len,
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    parse_curve(&buf[..len as usize])
}

pub fn ballistics() -> Ballistics {
    let mut mouse = [0i32; 3];
    let mut speed: u32 = 10;
    unsafe {
        SystemParametersInfoW(SPI_GETMOUSE, 0, mouse.as_mut_ptr().cast(), 0);
        SystemParametersInfoW(SPI_GETMOUSESPEED, 0, (&mut speed as *mut u32).cast(), 0);
    }
    if mouse[2] == 0 {
        log_info!("velocidade do mouse: {speed}, sem precisão aprimorada");
        return Ballistics::linear(speed);
    }
    let x = curve("SmoothMouseXCurve");
    let y = curve("SmoothMouseYCurve");
    let dpi = unsafe { GetDpiForSystem() };
    log_info!(
        "velocidade do mouse: {speed}, precisão aprimorada, DPI {dpi}, curva {}",
        if x.is_some() && y.is_some() {
            "do registro"
        } else {
            "padrão"
        }
    );
    Ballistics::enhanced(
        speed,
        x.unwrap_or(DEFAULT_CURVE_X),
        y.unwrap_or(DEFAULT_CURVE_Y),
        dpi,
    )
}
