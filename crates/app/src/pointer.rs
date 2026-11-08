use std::path::PathBuf;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{mpsc, OnceLock};
use std::{fs, ptr, thread};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateCursor, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetCursorPos, GetMessageW,
    GetSystemMetrics, PostMessageW, RegisterClassW, SetCursor, SetCursorPos,
    SetLayeredWindowAttributes, SetWindowPos, ShowWindow, SystemParametersInfoW, HCURSOR,
    HWND_TOPMOST, LWA_ALPHA, MA_NOACTIVATE, MSG, SM_CXCURSOR, SM_CYCURSOR, SPI_SETCURSORS,
    SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE, WM_APP, WM_MOUSEACTIVATE, WM_SETCURSOR, WNDCLASSW,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

const VEIL_SIDE: i32 = 24;
const WM_VEIL: u32 = WM_APP + 0x40;
const COVER: WPARAM = 1;
const UNCOVER: WPARAM = 0;

static VEIL: OnceLock<Option<isize>> = OnceLock::new();
static BLANK: AtomicIsize = AtomicIsize::new(0);

pub fn hide() {
    post(COVER);
}

pub fn show() {
    post(UNCOVER);
}

fn post(action: WPARAM) {
    let window = VEIL.get_or_init(spawn_veil).unwrap_or(0);
    if window != 0 {
        unsafe { PostMessageW(window as HWND, WM_VEIL, action, 0) };
    }
}

fn spawn_veil() -> Option<isize> {
    let (tx, rx) = mpsc::channel();
    thread::Builder::new()
        .name("veil".to_string())
        .spawn(move || unsafe { veil_thread(tx) })
        .ok()?;
    rx.recv().ok().flatten()
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

fn mask_bytes(width: i32, height: i32) -> usize {
    ((width as u32).div_ceil(16) * 2 * height as u32) as usize
}

unsafe fn transparent_cursor() -> HCURSOR {
    let width = GetSystemMetrics(SM_CXCURSOR).max(1);
    let height = GetSystemMetrics(SM_CYCURSOR).max(1);
    let bytes = mask_bytes(width, height);
    let keep_screen = vec![u8::MAX; bytes];
    let no_ink = vec![0u8; bytes];
    CreateCursor(
        GetModuleHandleW(ptr::null()),
        0,
        0,
        width,
        height,
        keep_screen.as_ptr().cast(),
        no_ink.as_ptr().cast(),
    )
}

unsafe fn veil_thread(ready: mpsc::Sender<Option<isize>>) {
    let module = GetModuleHandleW(ptr::null());
    let blank = transparent_cursor();
    BLANK.store(blank as isize, Ordering::SeqCst);
    let class = wide("lan-kvm-veil");
    let wc = WNDCLASSW {
        lpfnWndProc: Some(veil_proc),
        hInstance: module,
        hCursor: blank,
        lpszClassName: class.as_ptr(),
        ..std::mem::zeroed()
    };
    RegisterClassW(&wc);
    let window = CreateWindowExW(
        WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST,
        class.as_ptr(),
        ptr::null(),
        WS_POPUP,
        0,
        0,
        VEIL_SIDE,
        VEIL_SIDE,
        ptr::null_mut(),
        ptr::null_mut(),
        module,
        ptr::null(),
    );
    if window.is_null() {
        log_warn!("não foi possível criar a janela que esconde o cursor");
        let _ = ready.send(None);
        return;
    }
    SetLayeredWindowAttributes(window, 0, 1, LWA_ALPHA);
    let _ = ready.send(Some(window as isize));
    let mut msg: MSG = std::mem::zeroed();
    while GetMessageW(&mut msg, ptr::null_mut(), 0, 0) > 0 {
        DispatchMessageW(&msg);
    }
}

unsafe fn nudge_cursor() -> POINT {
    let mut at = POINT { x: 0, y: 0 };
    GetCursorPos(&mut at);
    SetCursorPos(at.x, at.y);
    at
}

unsafe extern "system" fn veil_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_VEIL if wparam == COVER => {
            let mut at = POINT { x: 0, y: 0 };
            GetCursorPos(&mut at);
            SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                at.x - VEIL_SIDE / 2,
                at.y - VEIL_SIDE / 2,
                VEIL_SIDE,
                VEIL_SIDE,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            nudge_cursor();
            0
        }
        WM_VEIL => {
            ShowWindow(hwnd, SW_HIDE);
            nudge_cursor();
            0
        }
        WM_SETCURSOR => {
            SetCursor(BLANK.load(Ordering::SeqCst) as HCURSOR);
            1
        }
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn old_hide_marker() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(|dir| PathBuf::from(dir).join("lan-kvm").join("pointer-hidden"))
}

pub fn undo_old_hide() {
    let Some(marker) = old_hide_marker().filter(|m| m.exists()) else {
        return;
    };
    log_warn!("cursores do sistema recarregados: uma versão anterior terminou com eles escondidos");
    unsafe { SystemParametersInfoW(SPI_SETCURSORS, 0, ptr::null_mut(), 0) };
    let _ = fs::remove_file(marker);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_masks_cover_whole_rows_of_words() {
        assert_eq!(mask_bytes(32, 32), 128);
        assert_eq!(mask_bytes(48, 48), 288);
        assert_eq!(mask_bytes(20, 3), 12);
        assert_eq!(mask_bytes(1, 1), 2);
    }
}
