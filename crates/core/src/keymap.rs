pub const KEY_NUMLOCK: u16 = 69;
pub const KEY_PAUSE: u16 = 119;

pub fn evdev_from_windows_scancode(scan: u32, extended: bool) -> Option<u16> {
    if scan > 0xFF {
        return None;
    }
    let scan = scan as u16;
    if !extended {
        return match scan {
            0x01..=0x58 => Some(scan),
            0x64..=0x6E => Some(183 + scan - 0x64),
            0x70 => Some(93),
            0x73 => Some(89),
            0x76 => Some(194),
            0x79 => Some(92),
            0x7B => Some(94),
            0x7D => Some(124),
            0x7E => Some(121),
            _ => None,
        };
    }
    Some(match scan {
        0x10 => 165,
        0x19 => 163,
        0x1C => 96,
        0x1D => 97,
        0x20 => 113,
        0x21 => 140,
        0x22 => 164,
        0x24 => 166,
        0x2E => 114,
        0x30 => 115,
        0x32 => 172,
        0x35 => 98,
        0x37 => 99,
        0x38 => 100,
        0x45 => KEY_NUMLOCK,
        0x47 => 102,
        0x48 => 103,
        0x49 => 104,
        0x4B => 105,
        0x4D => 106,
        0x4F => 107,
        0x50 => 108,
        0x51 => 109,
        0x52 => 110,
        0x53 => 111,
        0x5B => 125,
        0x5C => 126,
        0x5D => 127,
        0x5E => 116,
        0x5F => 142,
        0x63 => 143,
        0x65 => 217,
        0x66 => 156,
        0x67 => 173,
        0x68 => 128,
        0x69 => 159,
        0x6A => 158,
        0x6B => 157,
        0x6C => 155,
        0x6D => 226,
        _ => return None,
    })
}

pub fn windows_scancode_from_evdev(code: u16) -> Option<(u32, bool)> {
    [false, true]
        .into_iter()
        .flat_map(|extended| (1..=0xFFu32).map(move |scan| (scan, extended)))
        .find(|&(scan, extended)| evdev_from_windows_scancode(scan, extended) == Some(code))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reverse_map_roundtrips_every_known_key() {
        for extended in [false, true] {
            for scan in 1..=0xFFu32 {
                if let Some(code) = evdev_from_windows_scancode(scan, extended) {
                    let (s, e) = windows_scancode_from_evdev(code).unwrap();
                    assert_eq!(evdev_from_windows_scancode(s, e), Some(code));
                }
            }
        }
    }

    #[test]
    fn reverse_map_prefers_plain_scancode_and_rejects_unknown() {
        assert_eq!(windows_scancode_from_evdev(30), Some((0x1E, false)));
        assert_eq!(windows_scancode_from_evdev(97), Some((0x1D, true)));
        assert_eq!(
            windows_scancode_from_evdev(KEY_NUMLOCK),
            Some((0x45, false))
        );
        assert_eq!(windows_scancode_from_evdev(KEY_PAUSE), None);
        assert_eq!(windows_scancode_from_evdev(0), None);
    }

    #[test]
    fn main_block_matches_evdev() {
        assert_eq!(evdev_from_windows_scancode(0x01, false), Some(1));
        assert_eq!(evdev_from_windows_scancode(0x1E, false), Some(30));
        assert_eq!(evdev_from_windows_scancode(0x2A, false), Some(42));
        assert_eq!(evdev_from_windows_scancode(0x56, false), Some(86));
        assert_eq!(evdev_from_windows_scancode(0x58, false), Some(88));
    }

    #[test]
    fn extended_keys_differ_from_their_plain_twins() {
        assert_eq!(evdev_from_windows_scancode(0x1D, false), Some(29));
        assert_eq!(evdev_from_windows_scancode(0x1D, true), Some(97));
        assert_eq!(evdev_from_windows_scancode(0x38, false), Some(56));
        assert_eq!(evdev_from_windows_scancode(0x38, true), Some(100));
        assert_eq!(evdev_from_windows_scancode(0x1C, true), Some(96));
        assert_eq!(evdev_from_windows_scancode(0x48, false), Some(72));
        assert_eq!(evdev_from_windows_scancode(0x48, true), Some(103));
        assert_eq!(evdev_from_windows_scancode(0x5B, true), Some(125));
        assert_eq!(evdev_from_windows_scancode(0x45, true), Some(KEY_NUMLOCK));
    }

    #[test]
    fn abnt2_and_function_keys() {
        assert_eq!(evdev_from_windows_scancode(0x73, false), Some(89));
        assert_eq!(evdev_from_windows_scancode(0x7E, false), Some(121));
        assert_eq!(evdev_from_windows_scancode(0x64, false), Some(183));
        assert_eq!(evdev_from_windows_scancode(0x6E, false), Some(193));
        assert_eq!(evdev_from_windows_scancode(0x76, false), Some(194));
    }

    #[test]
    fn synthetic_and_unknown_are_ignored() {
        assert_eq!(evdev_from_windows_scancode(0x21D, false), None);
        assert_eq!(evdev_from_windows_scancode(0x2A, true), None);
        assert_eq!(evdev_from_windows_scancode(0x36, true), None);
        assert_eq!(evdev_from_windows_scancode(0x00, false), None);
        assert_eq!(evdev_from_windows_scancode(0x60, false), None);
    }
}
