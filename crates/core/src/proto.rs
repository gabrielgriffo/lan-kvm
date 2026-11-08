use std::fmt;

use crate::switch::Rect;

pub const PROTOCOL_VERSION: u16 = 13;
const MAX_SCREENS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
    Middle,
    Back,
    Forward,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    Hello {
        version: u16,
        width: u16,
        height: u16,
    },
    Enter {
        x: u16,
        y: u16,
    },
    Motion {
        x: u16,
        y: u16,
    },
    Button {
        button: Button,
        pressed: bool,
    },
    Wheel {
        delta: i16,
        horizontal: bool,
    },
    Key {
        code: u16,
        pressed: bool,
    },
    Leave,
    Ping,
    Pong,
    Clipboard(String),
    Layout {
        remote_on_right: bool,
    },
    TakeOver {
        y: u16,
    },
    Delta {
        dx: i16,
        dy: i16,
    },
    GiveBack {
        y: u16,
    },
    Abort,
    MonitorSetup {
        enabled: bool,
        pc_input: u8,
        notebook_input: u8,
        delay_ms: u16,
        name: String,
    },
    MonitorReport {
        ok: bool,
        detail: String,
    },
    MonitorSwitch {
        value: u8,
        name: String,
    },
    AudioSetup {
        enabled: bool,
    },
    Speaker(Vec<u8>),
    Mic(Vec<u8>),
    AudioReport {
        ok: bool,
        detail: String,
    },
    Screens(Vec<(Rect, String)>),
    TapSetup {
        pc_input: u8,
        name: String,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub enum DecodeError {
    Empty,
    UnknownTag(u8),
    UnknownButton(u8),
    BadLength { tag: u8, len: usize },
    InvalidUtf8,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::Empty => write!(f, "mensagem vazia"),
            DecodeError::UnknownTag(t) => write!(f, "tipo de mensagem desconhecido: {t}"),
            DecodeError::UnknownButton(b) => write!(f, "botão desconhecido: {b}"),
            DecodeError::BadLength { tag, len } => {
                write!(f, "tamanho {len} inválido para a mensagem {tag}")
            }
            DecodeError::InvalidUtf8 => write!(f, "texto do clipboard não é UTF-8 válido"),
        }
    }
}

impl std::error::Error for DecodeError {}

const HELLO: u8 = 1;
const ENTER: u8 = 2;
const MOTION: u8 = 3;
const BUTTON: u8 = 4;
const WHEEL: u8 = 5;
const LEAVE: u8 = 6;
const KEY: u8 = 7;
const PING: u8 = 8;
const PONG: u8 = 9;
const CLIPBOARD: u8 = 10;
const LAYOUT: u8 = 11;
const TAKE_OVER: u8 = 12;
const DELTA: u8 = 13;
const GIVE_BACK: u8 = 14;
const ABORT: u8 = 15;
const MONITOR_SETUP: u8 = 16;
const MONITOR_REPORT: u8 = 17;
const MONITOR_SWITCH: u8 = 18;
const AUDIO_SETUP: u8 = 19;
const SPEAKER: u8 = 20;
const MIC: u8 = 21;
const AUDIO_REPORT: u8 = 22;
const SCREENS: u8 = 23;
const TAP_SETUP: u8 = 24;

impl Button {
    fn code(self) -> u8 {
        match self {
            Button::Left => 1,
            Button::Right => 2,
            Button::Middle => 3,
            Button::Back => 4,
            Button::Forward => 5,
        }
    }

    fn from_code(code: u8) -> Result<Self, DecodeError> {
        Ok(match code {
            1 => Button::Left,
            2 => Button::Right,
            3 => Button::Middle,
            4 => Button::Back,
            5 => Button::Forward,
            _ => return Err(DecodeError::UnknownButton(code)),
        })
    }
}

impl Message {
    pub fn encode(&self, out: &mut Vec<u8>) {
        match *self {
            Message::Hello {
                version,
                width,
                height,
            } => {
                out.push(HELLO);
                out.extend_from_slice(&version.to_be_bytes());
                out.extend_from_slice(&width.to_be_bytes());
                out.extend_from_slice(&height.to_be_bytes());
            }
            Message::Enter { x, y } => {
                out.push(ENTER);
                out.extend_from_slice(&x.to_be_bytes());
                out.extend_from_slice(&y.to_be_bytes());
            }
            Message::Motion { x, y } => {
                out.push(MOTION);
                out.extend_from_slice(&x.to_be_bytes());
                out.extend_from_slice(&y.to_be_bytes());
            }
            Message::Button { button, pressed } => {
                out.push(BUTTON);
                out.push(button.code());
                out.push(pressed as u8);
            }
            Message::Wheel { delta, horizontal } => {
                out.push(WHEEL);
                out.extend_from_slice(&delta.to_be_bytes());
                out.push(horizontal as u8);
            }
            Message::Key { code, pressed } => {
                out.push(KEY);
                out.extend_from_slice(&code.to_be_bytes());
                out.push(pressed as u8);
            }
            Message::Leave => out.push(LEAVE),
            Message::Ping => out.push(PING),
            Message::Pong => out.push(PONG),
            Message::Clipboard(ref text) => {
                out.push(CLIPBOARD);
                out.extend_from_slice(text.as_bytes());
            }
            Message::Layout { remote_on_right } => {
                out.push(LAYOUT);
                out.push(remote_on_right as u8);
            }
            Message::TakeOver { y } => {
                out.push(TAKE_OVER);
                out.extend_from_slice(&y.to_be_bytes());
            }
            Message::Delta { dx, dy } => {
                out.push(DELTA);
                out.extend_from_slice(&dx.to_be_bytes());
                out.extend_from_slice(&dy.to_be_bytes());
            }
            Message::GiveBack { y } => {
                out.push(GIVE_BACK);
                out.extend_from_slice(&y.to_be_bytes());
            }
            Message::Abort => out.push(ABORT),
            Message::MonitorSetup {
                enabled,
                pc_input,
                notebook_input,
                delay_ms,
                ref name,
            } => {
                out.extend_from_slice(&[MONITOR_SETUP, enabled as u8, pc_input, notebook_input]);
                out.extend_from_slice(&delay_ms.to_be_bytes());
                out.extend_from_slice(name.as_bytes());
            }
            Message::MonitorReport { ok, ref detail } => {
                out.extend_from_slice(&[MONITOR_REPORT, ok as u8]);
                out.extend_from_slice(detail.as_bytes());
            }
            Message::MonitorSwitch { value, ref name } => {
                out.extend_from_slice(&[MONITOR_SWITCH, value]);
                out.extend_from_slice(name.as_bytes());
            }
            Message::AudioSetup { enabled } => out.extend_from_slice(&[AUDIO_SETUP, enabled as u8]),
            Message::Speaker(ref pcm) => {
                out.push(SPEAKER);
                out.extend_from_slice(pcm);
            }
            Message::Mic(ref pcm) => {
                out.push(MIC);
                out.extend_from_slice(pcm);
            }
            Message::AudioReport { ok, ref detail } => {
                out.extend_from_slice(&[AUDIO_REPORT, ok as u8]);
                out.extend_from_slice(detail.as_bytes());
            }
            Message::TapSetup { pc_input, ref name } => {
                out.extend_from_slice(&[TAP_SETUP, pc_input]);
                out.extend_from_slice(name.as_bytes());
            }
            Message::Screens(ref screens) => {
                out.push(SCREENS);
                for (r, name) in screens.iter().take(MAX_SCREENS) {
                    for v in [r.x, r.y, r.w, r.h] {
                        out.extend_from_slice(&(v.clamp(-32768, 32767) as i16).to_be_bytes());
                    }
                    let name = &name.as_bytes()[..name.len().min(255)];
                    out.push(name.len() as u8);
                    out.extend_from_slice(name);
                }
            }
        }
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let (&tag, body) = bytes.split_first().ok_or(DecodeError::Empty)?;
        if tag == CLIPBOARD {
            return String::from_utf8(body.to_vec())
                .map(Message::Clipboard)
                .map_err(|_| DecodeError::InvalidUtf8);
        }
        let text = |from: usize| {
            String::from_utf8(body[from..].to_vec()).map_err(|_| DecodeError::InvalidUtf8)
        };
        if tag == MONITOR_SETUP {
            if body.len() < 5 {
                return Err(DecodeError::BadLength {
                    tag,
                    len: body.len(),
                });
            }
            return Ok(Message::MonitorSetup {
                enabled: body[0] != 0,
                pc_input: body[1],
                notebook_input: body[2],
                delay_ms: u16::from_be_bytes([body[3], body[4]]),
                name: text(5)?,
            });
        }
        if tag == SCREENS {
            return decode_screens(body).map(Message::Screens);
        }
        match tag {
            SPEAKER => return Ok(Message::Speaker(body.to_vec())),
            MIC => return Ok(Message::Mic(body.to_vec())),
            _ => {}
        }
        if matches!(
            tag,
            MONITOR_REPORT | MONITOR_SWITCH | AUDIO_REPORT | TAP_SETUP
        ) {
            if body.is_empty() {
                return Err(DecodeError::BadLength { tag, len: 0 });
            }
            return Ok(match tag {
                MONITOR_REPORT => Message::MonitorReport {
                    ok: body[0] != 0,
                    detail: text(1)?,
                },
                AUDIO_REPORT => Message::AudioReport {
                    ok: body[0] != 0,
                    detail: text(1)?,
                },
                TAP_SETUP => Message::TapSetup {
                    pc_input: body[0],
                    name: text(1)?,
                },
                _ => Message::MonitorSwitch {
                    value: body[0],
                    name: text(1)?,
                },
            });
        }
        let expected = match tag {
            HELLO => 6,
            ENTER | MOTION => 4,
            BUTTON => 2,
            WHEEL | KEY => 3,
            LEAVE | PING | PONG | ABORT => 0,
            LAYOUT | AUDIO_SETUP => 1,
            TAKE_OVER | GIVE_BACK => 2,
            DELTA => 4,
            _ => return Err(DecodeError::UnknownTag(tag)),
        };
        if body.len() != expected {
            return Err(DecodeError::BadLength {
                tag,
                len: body.len(),
            });
        }
        let u16_at = |i: usize| u16::from_be_bytes([body[i], body[i + 1]]);
        Ok(match tag {
            HELLO => Message::Hello {
                version: u16_at(0),
                width: u16_at(2),
                height: u16_at(4),
            },
            ENTER => Message::Enter {
                x: u16_at(0),
                y: u16_at(2),
            },
            MOTION => Message::Motion {
                x: u16_at(0),
                y: u16_at(2),
            },
            BUTTON => Message::Button {
                button: Button::from_code(body[0])?,
                pressed: body[1] != 0,
            },
            WHEEL => Message::Wheel {
                delta: u16_at(0) as i16,
                horizontal: body[2] != 0,
            },
            KEY => Message::Key {
                code: u16_at(0),
                pressed: body[2] != 0,
            },
            PING => Message::Ping,
            LAYOUT => Message::Layout {
                remote_on_right: body[0] != 0,
            },
            TAKE_OVER => Message::TakeOver { y: u16_at(0) },
            DELTA => Message::Delta {
                dx: u16_at(0) as i16,
                dy: u16_at(2) as i16,
            },
            GIVE_BACK => Message::GiveBack { y: u16_at(0) },
            ABORT => Message::Abort,
            AUDIO_SETUP => Message::AudioSetup {
                enabled: body[0] != 0,
            },
            PONG => Message::Pong,
            _ => Message::Leave,
        })
    }
}

fn decode_screens(mut body: &[u8]) -> Result<Vec<(Rect, String)>, DecodeError> {
    let bad = |len| DecodeError::BadLength { tag: SCREENS, len };
    let total = body.len();
    let mut screens = Vec::new();
    while !body.is_empty() {
        if body.len() < 9 || screens.len() >= MAX_SCREENS {
            return Err(bad(total));
        }
        let v = |i: usize| i16::from_be_bytes([body[i], body[i + 1]]) as i32;
        let rect = Rect {
            x: v(0),
            y: v(2),
            w: v(4),
            h: v(6),
        };
        let len = body[8] as usize;
        let name = body.get(9..9 + len).ok_or(bad(total))?;
        let name = String::from_utf8(name.to_vec()).map_err(|_| DecodeError::InvalidUtf8)?;
        screens.push((rect, name));
        body = &body[9 + len..];
    }
    Ok(screens)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(msg: Message) {
        let mut buf = Vec::new();
        msg.encode(&mut buf);
        assert_eq!(Message::decode(&buf), Ok(msg));
    }

    #[test]
    fn tap_setup_roundtrip_and_needs_the_input() {
        roundtrip(Message::TapSetup {
            pc_input: 0x0F,
            name: "34GL750".to_string(),
        });
        roundtrip(Message::TapSetup {
            pc_input: 0,
            name: String::new(),
        });
        assert_eq!(
            Message::decode(&[TAP_SETUP]),
            Err(DecodeError::BadLength {
                tag: TAP_SETUP,
                len: 0
            })
        );
    }

    #[test]
    fn screens_roundtrip_and_reject_truncated_bodies() {
        let lg = Rect {
            x: 0,
            y: 0,
            w: 2560,
            h: 1080,
        };
        let laptop = Rect {
            x: 2560,
            y: 0,
            w: 1920,
            h: 1080,
        };
        roundtrip(Message::Screens(vec![
            (lg, "34GL750".to_string()),
            (laptop, String::new()),
        ]));
        roundtrip(Message::Screens(Vec::new()));
        let mut buf = Vec::new();
        Message::Screens(vec![(lg, "34GL750".to_string())]).encode(&mut buf);
        for cut in 1..buf.len() - 1 {
            assert!(Message::decode(&buf[..=cut]).is_err(), "corte em {cut}");
        }
    }

    #[test]
    fn roundtrip_all_variants() {
        roundtrip(Message::Hello {
            version: PROTOCOL_VERSION,
            width: 1920,
            height: 1080,
        });
        roundtrip(Message::Enter { x: 0, y: 540 });
        roundtrip(Message::Motion { x: u16::MAX, y: 1 });
        for button in [
            Button::Left,
            Button::Right,
            Button::Middle,
            Button::Back,
            Button::Forward,
        ] {
            roundtrip(Message::Button {
                button,
                pressed: true,
            });
            roundtrip(Message::Button {
                button,
                pressed: false,
            });
        }
        roundtrip(Message::Wheel {
            delta: -120,
            horizontal: false,
        });
        roundtrip(Message::Wheel {
            delta: 240,
            horizontal: true,
        });
        roundtrip(Message::Key {
            code: 125,
            pressed: true,
        });
        roundtrip(Message::Key {
            code: 30,
            pressed: false,
        });
        roundtrip(Message::Leave);
        roundtrip(Message::Ping);
        roundtrip(Message::Pong);
        roundtrip(Message::Clipboard(String::new()));
        roundtrip(Message::Layout {
            remote_on_right: true,
        });
        roundtrip(Message::Layout {
            remote_on_right: false,
        });
        roundtrip(Message::TakeOver { y: 1000 });
        roundtrip(Message::Delta { dx: -5, dy: 32767 });
        roundtrip(Message::Delta {
            dx: i16::MIN,
            dy: 0,
        });
        roundtrip(Message::GiveBack { y: 0 });
        roundtrip(Message::Abort);
        roundtrip(Message::MonitorSetup {
            enabled: true,
            pc_input: 0x0F,
            notebook_input: 0x12,
            delay_ms: 3000,
            name: "34GL750".to_string(),
        });
        roundtrip(Message::MonitorSetup {
            enabled: false,
            pc_input: 0,
            notebook_input: 0,
            delay_ms: 0,
            name: String::new(),
        });
        roundtrip(Message::MonitorReport {
            ok: false,
            detail: "sem permissão em /dev/i2c-6".to_string(),
        });
        roundtrip(Message::MonitorSwitch {
            value: 0x11,
            name: "LG ULTRAGEAR".to_string(),
        });
        roundtrip(Message::AudioSetup { enabled: true });
        roundtrip(Message::AudioSetup { enabled: false });
        roundtrip(Message::Speaker(vec![1, 2, 3, 4]));
        roundtrip(Message::Mic(Vec::new()));
        roundtrip(Message::AudioReport {
            ok: true,
            detail: "dispositivos virtuais criados".to_string(),
        });
        roundtrip(Message::Clipboard("ação\nlinha 2 ✓".to_string()));
    }

    #[test]
    fn rejects_empty_and_unknown() {
        assert_eq!(Message::decode(&[]), Err(DecodeError::Empty));
        assert_eq!(Message::decode(&[99]), Err(DecodeError::UnknownTag(99)));
        assert_eq!(
            Message::decode(&[BUTTON, 9, 1]),
            Err(DecodeError::UnknownButton(9))
        );
    }

    #[test]
    fn rejects_short_monitor_messages() {
        assert_eq!(
            Message::decode(&[MONITOR_SETUP, 1, 0x0F]),
            Err(DecodeError::BadLength {
                tag: MONITOR_SETUP,
                len: 2
            })
        );
        assert_eq!(
            Message::decode(&[MONITOR_REPORT]),
            Err(DecodeError::BadLength {
                tag: MONITOR_REPORT,
                len: 0
            })
        );
        assert_eq!(
            Message::decode(&[AUDIO_REPORT]),
            Err(DecodeError::BadLength {
                tag: AUDIO_REPORT,
                len: 0
            })
        );
        assert_eq!(
            Message::decode(&[AUDIO_SETUP]),
            Err(DecodeError::BadLength {
                tag: AUDIO_SETUP,
                len: 0
            })
        );
        assert_eq!(
            Message::decode(&[MONITOR_SWITCH]),
            Err(DecodeError::BadLength {
                tag: MONITOR_SWITCH,
                len: 0
            })
        );
        assert_eq!(
            Message::decode(&[MONITOR_SETUP, 1, 1, 1, 0x0B]),
            Err(DecodeError::BadLength {
                tag: MONITOR_SETUP,
                len: 4
            })
        );
        assert_eq!(
            Message::decode(&[MONITOR_SETUP, 1, 1, 1, 0x0B, 0xB8, 0xFF]),
            Err(DecodeError::InvalidUtf8)
        );
    }

    #[test]
    fn rejects_invalid_utf8_clipboard() {
        assert_eq!(
            Message::decode(&[CLIPBOARD, 0xff, 0xfe]),
            Err(DecodeError::InvalidUtf8)
        );
    }

    #[test]
    fn rejects_wrong_length() {
        assert_eq!(
            Message::decode(&[MOTION, 0, 1, 2]),
            Err(DecodeError::BadLength {
                tag: MOTION,
                len: 3
            })
        );
        assert_eq!(
            Message::decode(&[LEAVE, 0]),
            Err(DecodeError::BadLength { tag: LEAVE, len: 1 })
        );
    }
}
