use std::mem;

use lan_kvm_core::keymap::{windows_scancode_from_evdev, KEY_PAUSE};
use lan_kvm_core::proto::Button;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    XBUTTON1, XBUTTON2,
};

pub const MAGIC: usize = 0x4C4B_564D;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Injection {
    MoveTo(i32, i32),
    Button(Button, bool),
    Wheel(i16, bool),
    Key(u16, bool),
}

pub fn send(list: &[Injection]) {
    let inputs: Vec<INPUT> = list.iter().filter_map(to_input).collect();
    if inputs.is_empty() {
        return;
    }
    unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            mem::size_of::<INPUT>() as i32,
        )
    };
}

fn normalize(offset: i32, len: i32) -> i32 {
    if len <= 1 {
        return 0;
    }
    let span = (len - 1) as i64;
    ((offset.clamp(0, len - 1) as i64 * 65535 + span / 2) / span) as i32
}

fn mouse(dx: i32, dy: i32, data: u32, flags: MOUSE_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: data,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: MAGIC,
            },
        },
    }
}

fn key(vk: VIRTUAL_KEY, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: MAGIC,
            },
        },
    }
}

fn to_input(injection: &Injection) -> Option<INPUT> {
    Some(match *injection {
        Injection::MoveTo(x, y) => {
            let (vx, vy, vw, vh) = unsafe {
                (
                    GetSystemMetrics(SM_XVIRTUALSCREEN),
                    GetSystemMetrics(SM_YVIRTUALSCREEN),
                    GetSystemMetrics(SM_CXVIRTUALSCREEN),
                    GetSystemMetrics(SM_CYVIRTUALSCREEN),
                )
            };
            mouse(
                normalize(x - vx, vw),
                normalize(y - vy, vh),
                0,
                MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
            )
        }
        Injection::Button(button, pressed) => {
            let (flags, data) = match (button, pressed) {
                (Button::Left, true) => (MOUSEEVENTF_LEFTDOWN, 0),
                (Button::Left, false) => (MOUSEEVENTF_LEFTUP, 0),
                (Button::Right, true) => (MOUSEEVENTF_RIGHTDOWN, 0),
                (Button::Right, false) => (MOUSEEVENTF_RIGHTUP, 0),
                (Button::Middle, true) => (MOUSEEVENTF_MIDDLEDOWN, 0),
                (Button::Middle, false) => (MOUSEEVENTF_MIDDLEUP, 0),
                (Button::Back, true) => (MOUSEEVENTF_XDOWN, XBUTTON1 as u32),
                (Button::Back, false) => (MOUSEEVENTF_XUP, XBUTTON1 as u32),
                (Button::Forward, true) => (MOUSEEVENTF_XDOWN, XBUTTON2 as u32),
                (Button::Forward, false) => (MOUSEEVENTF_XUP, XBUTTON2 as u32),
            };
            mouse(0, 0, data, flags)
        }
        Injection::Wheel(delta, horizontal) => {
            let flags = if horizontal {
                MOUSEEVENTF_HWHEEL
            } else {
                MOUSEEVENTF_WHEEL
            };
            mouse(0, 0, delta as i32 as u32, flags)
        }
        Injection::Key(code, pressed) => {
            let up = if pressed { 0 } else { KEYEVENTF_KEYUP };
            if code == KEY_PAUSE {
                key(VK_PAUSE, 0, up)
            } else {
                let (scan, extended) = windows_scancode_from_evdev(code)?;
                let ext = if extended { KEYEVENTF_EXTENDEDKEY } else { 0 };
                key(0, scan as u16, KEYEVENTF_SCANCODE | ext | up)
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_covers_full_range() {
        assert_eq!(normalize(0, 8320), 0);
        assert_eq!(normalize(8319, 8320), 65535);
        assert_eq!(normalize(-5, 8320), 0);
        assert_eq!(normalize(99999, 8320), 65535);
        assert_eq!(normalize(4160, 8321), 32768);
        assert_eq!(normalize(3, 1), 0);
    }

    #[test]
    fn unknown_key_is_not_injected() {
        assert!(to_input(&Injection::Key(0, true)).is_none());
        assert!(to_input(&Injection::Key(30, true)).is_some());
        assert!(to_input(&Injection::Key(KEY_PAUSE, false)).is_some());
    }
}
