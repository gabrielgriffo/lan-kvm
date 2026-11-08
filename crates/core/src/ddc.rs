use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

pub const VCP_INPUT: u8 = 0x60;
pub const DDC_ADDRESS: u16 = 0x37;
const HOST_ADDRESS: u8 = 0x51;
const DEST_WRITE: u8 = 0x6E;

pub const SWITCH_DELAY: Duration = Duration::from_secs(3);

pub const STANDARD_INPUTS: [u8; 9] = [0x01, 0x03, 0x04, 0x0F, 0x10, 0x11, 0x12, 0x13, 0x1B];

pub fn input_name(value: u8) -> String {
    match value {
        0x01 => "VGA 1".into(),
        0x02 => "VGA 2".into(),
        0x03 => "DVI 1".into(),
        0x04 => "DVI 2".into(),
        0x0F => "DisplayPort 1".into(),
        0x10 => "DisplayPort 2".into(),
        0x11 => "HDMI 1".into(),
        0x12 => "HDMI 2".into(),
        0x13 => "HDMI 3".into(),
        0x1B => "USB-C".into(),
        other => format!("Entrada 0x{other:02X}"),
    }
}

pub fn set_vcp_packet(code: u8, value: u16) -> [u8; 7] {
    let [hi, lo] = value.to_be_bytes();
    let body = [HOST_ADDRESS, 0x84, 0x03, code, hi, lo];
    let checksum = body.iter().fold(DEST_WRITE, |acc, b| acc ^ b);
    [
        body[0], body[1], body[2], body[3], body[4], body[5], checksum,
    ]
}

pub fn input_values(capabilities: &str) -> Vec<u8> {
    let lower = capabilities.to_ascii_lowercase();
    let Some(vcp_start) = lower.find("vcp(") else {
        return Vec::new();
    };
    let body = &lower[vcp_start + 4..];
    let bytes = body.as_bytes();
    let mut depth = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' if depth == 0 => break,
            b')' => depth -= 1,
            _ if depth == 0 && body[i..].starts_with("60") => {
                let before_ok = i == 0 || !bytes[i - 1].is_ascii_hexdigit();
                let rest = body[i + 2..].trim_start();
                if before_ok && rest.starts_with('(') {
                    let inner_start = body.len() - rest.len() + 1;
                    let inner_end = body[inner_start..]
                        .find(')')
                        .map_or(body.len(), |e| inner_start + e);
                    return hex_values(&body[inner_start..inner_end]);
                }
            }
            _ => {}
        }
        i += 1;
    }
    Vec::new()
}

fn hex_values(text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for token in text.split(|c: char| !c.is_ascii_hexdigit()) {
        if token.is_empty() {
            continue;
        }
        let pairs: Vec<&str> = if token.len() <= 2 {
            vec![token]
        } else if token.len() % 2 == 0 {
            (0..token.len())
                .step_by(2)
                .map(|i| &token[i..i + 2])
                .collect()
        } else {
            continue;
        };
        for pair in pairs {
            if let Ok(value) = u8::from_str_radix(pair, 16) {
                if !out.contains(&value) {
                    out.push(value);
                }
            }
        }
    }
    out
}

pub fn edid_name(edid: &[u8]) -> Option<String> {
    if edid.len() < 128 {
        return None;
    }
    (54..126).step_by(18).find_map(|i| {
        let block = &edid[i..i + 18];
        if block[..3] != [0, 0, 0] || block[3] != 0xFC {
            return None;
        }
        let text: String = block[5..]
            .iter()
            .take_while(|&&b| b != b'\n')
            .map(|&b| b as char)
            .collect();
        let text = text.trim().to_string();
        (!text.is_empty()).then_some(text)
    })
}

pub fn same_monitor(a: &str, b: &str) -> bool {
    let norm = |s: &str| {
        s.chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect::<String>()
    };
    let (a, b) = (norm(a), norm(b));
    !a.is_empty() && (a == b || a.contains(&b) || b.contains(&a))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Switch {
        name: String,
        value: u8,
        delay: Duration,
    },
    Cancel,
    Flush,
}

#[derive(Debug, Default)]
pub struct Pending {
    job: Option<(String, u8, Instant)>,
}

impl Pending {
    pub fn handle(&mut self, request: Request, now: Instant) {
        match request {
            Request::Switch { name, value, delay } => {
                let deadline = now + delay;
                match &mut self.job {
                    Some((n, v, due)) if *n == name && *v == value => {
                        *due = (*due).min(deadline);
                    }
                    job => *job = Some((name, value, deadline)),
                }
            }
            Request::Cancel => self.job = None,
            Request::Flush => {
                if let Some((_, _, due)) = &mut self.job {
                    *due = (*due).min(now);
                }
            }
        }
    }

    pub fn take_due(&mut self, now: Instant) -> Option<(String, u8)> {
        match &self.job {
            Some((_, _, due)) if *due <= now => self.job.take().map(|(n, v, _)| (n, v)),
            _ => None,
        }
    }

    pub fn wait(&self, now: Instant) -> Option<Duration> {
        self.job
            .as_ref()
            .map(|(_, _, due)| due.saturating_duration_since(now))
    }
}

pub fn spawn_switcher<F>(mut apply: F) -> Sender<Request>
where
    F: FnMut(&str, u8) + Send + 'static,
{
    let (tx, rx) = mpsc::channel::<Request>();
    thread::spawn(move || {
        let mut pending = Pending::default();
        loop {
            if let Some((name, value)) = pending.take_due(Instant::now()) {
                apply(&name, value);
                continue;
            }
            let next = match pending.wait(Instant::now()) {
                Some(wait) => rx.recv_timeout(wait),
                None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
            };
            match next {
                Ok(request) => pending.handle(request, Instant::now()),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    });
    tx
}

#[cfg(test)]
mod tests {
    use super::*;

    fn switch(value: u8, secs: u64) -> Request {
        Request::Switch {
            name: "34GL750".into(),
            value,
            delay: Duration::from_secs(secs),
        }
    }

    #[test]
    fn switch_waits_the_whole_delay() {
        let t0 = Instant::now();
        let mut p = Pending::default();
        p.handle(switch(0x11, 3), t0);
        assert_eq!(p.take_due(t0 + Duration::from_millis(2999)), None);
        assert_eq!(
            p.take_due(t0 + Duration::from_secs(3)),
            Some(("34GL750".into(), 0x11))
        );
        assert_eq!(p.take_due(t0 + Duration::from_secs(10)), None);
    }

    #[test]
    fn coming_back_resets_instead_of_accumulating() {
        let t0 = Instant::now();
        let s = Duration::from_secs;
        let mut p = Pending::default();
        for round in 0..5 {
            let start = t0 + s(round * 3);
            p.handle(switch(0x11, 3), start);
            assert_eq!(p.take_due(start + s(2)), None);
            p.handle(Request::Cancel, start + s(2));
        }
        let start = t0 + s(15);
        p.handle(switch(0x11, 3), start);
        assert_eq!(p.take_due(start + s(2)), None);
        assert!(p.take_due(start + s(3)).is_some());
    }

    #[test]
    fn repeating_the_request_does_not_postpone_it() {
        let t0 = Instant::now();
        let mut p = Pending::default();
        p.handle(switch(0x11, 3), t0);
        p.handle(switch(0x11, 3), t0 + Duration::from_secs(2));
        assert!(p.take_due(t0 + Duration::from_secs(3)).is_some());
    }

    #[test]
    fn another_target_replaces_and_flush_fires_now() {
        let t0 = Instant::now();
        let mut p = Pending::default();
        p.handle(switch(0x11, 3), t0);
        p.handle(switch(0x0F, 3), t0 + Duration::from_secs(1));
        assert_eq!(p.take_due(t0 + Duration::from_secs(3)), None);
        p.handle(Request::Flush, t0 + Duration::from_secs(2));
        assert_eq!(
            p.take_due(t0 + Duration::from_secs(2)),
            Some(("34GL750".into(), 0x0F))
        );
        p.handle(Request::Flush, t0 + Duration::from_secs(2));
        assert_eq!(p.wait(t0), None);
    }

    #[test]
    fn zero_delay_is_immediate() {
        let t0 = Instant::now();
        let mut p = Pending::default();
        p.handle(switch(0x0F, 0), t0);
        assert!(p.take_due(t0).is_some());
    }

    #[test]
    fn switcher_thread_applies_only_uninterrupted_requests() {
        let (done_tx, done) = mpsc::channel();
        let tx =
            spawn_switcher(move |name, value| done_tx.send((name.to_string(), value)).unwrap());
        let request = |ms| Request::Switch {
            name: "lg".into(),
            value: 0x11,
            delay: Duration::from_millis(ms),
        };
        tx.send(request(300)).unwrap();
        thread::sleep(Duration::from_millis(100));
        tx.send(Request::Cancel).unwrap();
        assert!(done.recv_timeout(Duration::from_millis(500)).is_err());
        tx.send(request(100)).unwrap();
        assert_eq!(
            done.recv_timeout(Duration::from_secs(2)),
            Ok(("lg".into(), 0x11))
        );
        assert!(done.recv_timeout(Duration::from_millis(300)).is_err());
    }

    #[test]
    fn set_vcp_packet_has_ddc_ci_layout_and_checksum() {
        let p = set_vcp_packet(0x60, 0x12);
        assert_eq!(&p[..6], &[0x51, 0x84, 0x03, 0x60, 0x00, 0x12]);
        let expected = 0x6E ^ 0x51 ^ 0x84 ^ 0x03 ^ 0x60 ^ 0x12;
        assert_eq!(p[6], expected);
        assert_eq!(set_vcp_packet(0x10, 0x1234)[4..6], [0x12, 0x34]);
    }

    #[test]
    fn input_values_come_from_vcp_60() {
        let caps = "(prot(monitor)type(LCD)model(34GL750)cmds(01 02 03 0C E3 F3)vcp(02 04 05 08 10 12 14(05 08 0B) 16 18 1A 52 60( 11 12 0F) AC AE B2 B6 C0 C6 C8 C9 D6(01 04) DF 62 8D F4 F5(00 01 02) F6(00 01 02) 4D 4E 4F 15(01 06 11 13 14 28 29 32 48) F7(00 01 02 03) F8(00 01) F9 E4 E5 E6 E7 E8 E9 EA EB EF FD(00 01) FE(00 01 02) FF)mccs_ver(2.1)mswhql(1))";
        assert_eq!(input_values(caps), vec![0x11, 0x12, 0x0F]);
    }

    #[test]
    fn input_values_handle_packed_hex_and_absence() {
        assert_eq!(
            input_values("vcp(10 60(0f1112) 62)"),
            vec![0x0F, 0x11, 0x12]
        );
        assert_eq!(input_values("vcp(10 12 62)"), Vec::<u8>::new());
        assert_eq!(input_values("model(x)"), Vec::<u8>::new());
        assert_eq!(input_values("vcp(14(60 61) 60(11))"), vec![0x11]);
    }

    #[test]
    fn edid_monitor_name_descriptor() {
        let mut edid = vec![0u8; 128];
        edid[72..77].copy_from_slice(&[0, 0, 0, 0xFC, 0]);
        edid[77..90].copy_from_slice(b"34GL750\n     ");
        assert_eq!(edid_name(&edid), Some("34GL750".to_string()));
        assert_eq!(edid_name(&edid[..100]), None);
        assert_eq!(edid_name(&[0u8; 128]), None);
    }

    #[test]
    fn monitor_names_match_loosely() {
        assert!(same_monitor("34GL750", "LG 34GL750"));
        assert!(same_monitor("lg-34gl750", "34GL750"));
        assert!(!same_monitor("34GL750", "27UL500"));
        assert!(!same_monitor("", "34GL750"));
    }

    #[test]
    fn names_for_known_and_unknown_inputs() {
        assert_eq!(input_name(0x12), "HDMI 2");
        assert_eq!(input_name(0x0F), "DisplayPort 1");
        assert_eq!(input_name(0x90), "Entrada 0x90");
    }
}
