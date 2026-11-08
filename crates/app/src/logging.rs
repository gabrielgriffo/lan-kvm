use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender, SyncSender};
use std::sync::OnceLock;
use std::thread;

pub const MAX_BYTES: u64 = 1 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

impl Level {
    fn tag(self) -> &'static str {
        match self {
            Level::Info => "INFO",
            Level::Warn => "AVISO",
            Level::Error => "ERRO",
        }
    }
}

enum Entry {
    Line(String),
    Flush(SyncSender<()>),
}

static SINK: OnceLock<Sender<Entry>> = OnceLock::new();

macro_rules! log_info {
    ($($arg:tt)*) => { $crate::logging::write($crate::logging::Level::Info, format!($($arg)*)) };
}
macro_rules! log_warn {
    ($($arg:tt)*) => { $crate::logging::write($crate::logging::Level::Warn, format!($($arg)*)) };
}
macro_rules! log_error {
    ($($arg:tt)*) => { $crate::logging::write($crate::logging::Level::Error, format!($($arg)*)) };
}

pub fn path() -> Option<PathBuf> {
    if !cfg!(windows) {
        return None;
    }
    std::env::var_os("LOCALAPPDATA")
        .map(|dir| PathBuf::from(dir).join("lan-kvm").join("lan-kvm.log"))
}

pub fn init() {
    let Some(path) = path() else {
        return;
    };
    let (tx, rx) = mpsc::channel::<Entry>();
    if SINK.set(tx).is_err() {
        return;
    }
    let file_path = path.clone();
    thread::spawn(move || {
        for entry in rx {
            match entry {
                Entry::Line(line) => {
                    let _ = append_line(&file_path, &line, MAX_BYTES);
                }
                Entry::Flush(done) => {
                    let _ = done.send(());
                }
            }
        }
    });
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let line = format!("{} {} pânico: {info}", timestamp(), Level::Error.tag());
        let _ = append_line(&path, &line, MAX_BYTES);
        previous(info);
    }));
}

pub fn write(level: Level, message: String) {
    let line = format!("{} {} {message}", timestamp(), level.tag());
    if level == Level::Error {
        eprintln!("{line}");
    } else {
        println!("{line}");
    }
    if let Some(sink) = SINK.get() {
        let _ = sink.send(Entry::Line(line));
    }
}

pub fn flush() {
    if let Some(sink) = SINK.get() {
        let (done, wait) = mpsc::sync_channel(1);
        if sink.send(Entry::Flush(done)).is_ok() {
            let _ = wait.recv_timeout(std::time::Duration::from_secs(2));
        }
    }
}

pub fn append_line(path: &Path, line: &str, max: u64) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    if fs::metadata(path).is_ok_and(|m| m.len() >= max) {
        let old = path.with_extension("log.1");
        let _ = fs::remove_file(&old);
        fs::rename(path, &old)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{line}")
}

#[cfg(any(not(windows), test))]
pub fn civil(seconds: u64) -> (i64, u32, u32, u32, u32, u32) {
    let days = (seconds / 86_400) as i64;
    let rem = seconds % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (
        year,
        month,
        day,
        (rem / 3600) as u32,
        (rem / 60 % 60) as u32,
        (rem % 60) as u32,
    )
}

#[cfg(windows)]
fn timestamp() -> String {
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    let mut t = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut t) };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond, t.wMilliseconds
    )
}

#[cfg(not(windows))]
fn timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let (y, mo, d, h, mi, s) = civil(now.as_secs());
    format!(
        "{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}.{:03}Z",
        now.subsec_millis()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_date_from_unix_seconds() {
        assert_eq!(civil(0), (1970, 1, 1, 0, 0, 0));
        assert_eq!(civil(1_700_000_000), (2023, 11, 14, 22, 13, 20));
        assert_eq!(civil(951_782_400), (2000, 2, 29, 0, 0, 0));
        assert_eq!(civil(1_798_761_599), (2026, 12, 31, 23, 59, 59));
    }

    #[test]
    fn timestamp_has_fixed_shape() {
        let t = timestamp();
        assert!(t.starts_with("20"), "{t}");
        assert_eq!(&t[4..5], "-");
        assert_eq!(&t[10..11], " ");
        assert_eq!(&t[19..20], ".");
    }

    #[test]
    fn appends_and_rotates_when_too_big() {
        let dir = std::env::temp_dir().join(format!("lan-kvm-log-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("teste.log");
        append_line(&path, "primeira", 16).unwrap();
        append_line(&path, "segunda", 16).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "primeira\nsegunda\n");
        append_line(&path, "terceira", 16).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "terceira\n");
        assert_eq!(
            fs::read_to_string(path.with_extension("log.1")).unwrap(),
            "primeira\nsegunda\n"
        );
        append_line(&path, &"x".repeat(30), 16).unwrap();
        append_line(&path, "quinta", 16).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "quinta\n");
        assert!(fs::read_to_string(path.with_extension("log.1"))
            .unwrap()
            .contains("xxx"));
        fs::remove_dir_all(&dir).unwrap();
    }
}
