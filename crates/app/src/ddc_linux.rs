use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread::sleep;
use std::time::Duration;

use lan_kvm_core::ddc::{
    edid_name, input_name, same_monitor, set_vcp_packet, spawn_switcher, Request, DDC_ADDRESS,
    VCP_INPUT,
};
use lan_kvm_core::proto::Message;

const I2C_SLAVE: libc::c_ulong = 0x0703;
const DRM: &str = "/sys/class/drm";
const SETTLE: Duration = Duration::from_millis(50);

#[derive(Debug, Clone)]
pub struct Display {
    pub connector: String,
    pub name: String,
    pub device: Option<PathBuf>,
}

pub fn displays() -> Vec<Display> {
    let Ok(entries) = fs::read_dir(DRM) else {
        return Vec::new();
    };
    let mut out: Vec<Display> = entries
        .flatten()
        .filter_map(|entry| {
            let dir = entry.path();
            let connected =
                fs::read_to_string(dir.join("status")).is_ok_and(|s| s.trim() == "connected");
            if !connected {
                return None;
            }
            let name = fs::read(dir.join("edid"))
                .ok()
                .and_then(|edid| edid_name(&edid))?;
            let device = fs::read_link(dir.join("ddc"))
                .ok()
                .and_then(|link| link.file_name().map(|n| Path::new("/dev").join(n)));
            let connector = entry.file_name().to_string_lossy().into_owned();
            Some(Display {
                connector,
                name,
                device,
            })
        })
        .collect();
    out.sort_by(|a, b| a.connector.cmp(&b.connector));
    out
}

fn find(name: &str) -> Result<(Display, PathBuf), String> {
    let display = displays()
        .into_iter()
        .find(|d| same_monitor(&d.name, name))
        .ok_or_else(|| format!("monitor \"{name}\" não está conectado a este notebook"))?;
    let device = display
        .device
        .clone()
        .ok_or_else(|| format!("o conector {} não tem barramento DDC", display.connector))?;
    Ok((display, device))
}

fn open(device: &Path) -> Result<fs::File, String> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(device)
        .map_err(|e| match e.kind() {
            io::ErrorKind::PermissionDenied => format!(
                "sem permissão em {}; instale o pacote .deb do lan-kvm, que libera o DDC",
                device.display()
            ),
            _ => format!("não foi possível abrir {}: {e}", device.display()),
        })
}

pub fn check(name: &str) -> Result<String, String> {
    let (display, device) = find(name)?;
    open(&device)?;
    Ok(format!(
        "{} em {} ({})",
        display.name,
        display.connector,
        device.display()
    ))
}

pub fn set_input(name: &str, value: u8) -> Result<(), String> {
    let (display, device) = find(name)?;
    let mut file = open(&device)?;
    if unsafe { libc::ioctl(file.as_raw_fd(), I2C_SLAVE, DDC_ADDRESS as libc::c_ulong) } < 0 {
        return Err(format!(
            "ioctl I2C_SLAVE em {}: {}",
            device.display(),
            io::Error::last_os_error()
        ));
    }
    let packet = set_vcp_packet(VCP_INPUT, value as u16);
    let mut last = String::new();
    for attempt in 0..3 {
        if attempt > 0 {
            sleep(SETTLE);
        }
        match file.write_all(&packet) {
            Ok(()) => {
                sleep(SETTLE);
                return Ok(());
            }
            Err(e) => last = e.to_string(),
        }
    }
    Err(format!(
        "o monitor {} não aceitou a troca de entrada: {last}",
        display.name
    ))
}

pub fn switch_now(name: &str, value: u8) -> Message {
    let label = input_name(value);
    match set_input(name, value) {
        Ok(()) => {
            log_info!("duas batidas no topo: monitor {name} trocado para {label}");
            Message::MonitorReport {
                ok: true,
                detail: format!("duas batidas: trocou para {label}"),
            }
        }
        Err(detail) => {
            log_warn!("duas batidas no topo: não foi possível trocar para {label}: {detail}");
            Message::MonitorReport { ok: false, detail }
        }
    }
}

struct Setup {
    name: String,
    pc_input: u8,
    delay: Duration,
}

pub struct MonitorSwitch {
    setup: Mutex<Option<Setup>>,
    control_home: Mutex<Option<(String, u8)>>,
    tap_home: Mutex<Option<(String, u8)>>,
    showing_notebook: AtomicBool,
    worker: Mutex<Sender<Request>>,
}

impl MonitorSwitch {
    pub fn new() -> Arc<Self> {
        let worker = spawn_switcher(|name, value| {
            let label = input_name(value);
            match set_input(name, value) {
                Ok(()) => log_info!("monitor {name} trocado para {label}"),
                Err(e) => log_warn!("não foi possível trocar o monitor para {label}: {e}"),
            }
        });
        Arc::new(Self {
            setup: Mutex::new(None),
            control_home: Mutex::new(None),
            tap_home: Mutex::new(None),
            showing_notebook: AtomicBool::new(false),
            worker: Mutex::new(worker),
        })
    }

    fn request(&self, request: Request) {
        let _ = self.worker.lock().unwrap().send(request);
    }

    pub fn configure(
        &self,
        enabled: bool,
        name: String,
        pc_input: u8,
        delay: Duration,
    ) -> Option<Message> {
        self.request(Request::Cancel);
        if !enabled || name.is_empty() {
            *self.setup.lock().unwrap() = None;
            return None;
        }
        let report = match check(&name) {
            Ok(detail) => Message::MonitorReport { ok: true, detail },
            Err(detail) => {
                log_warn!("troca de monitor: {detail}");
                Message::MonitorReport { ok: false, detail }
            }
        };
        *self.control_home.lock().unwrap() = Some((name.clone(), pc_input));
        *self.setup.lock().unwrap() = Some(Setup {
            name,
            pc_input,
            delay,
        });
        Some(report)
    }

    pub fn set_tap_home(&self, name: String, pc_input: u8) {
        *self.tap_home.lock().unwrap() =
            (!name.is_empty() && pc_input != 0).then_some((name, pc_input));
    }

    pub fn go_home(&self) {
        let home = self
            .tap_home
            .lock()
            .unwrap()
            .clone()
            .or_else(|| self.control_home.lock().unwrap().clone());
        let Some((name, value)) = home else {
            return;
        };
        let label = input_name(value);
        match set_input(&name, value) {
            Ok(()) => log_info!("ao encerrar: pedido ao monitor {name} a entrada do PC ({label})"),
            Err(e) => log_warn!("ao encerrar: não foi possível pedir a entrada do PC: {e}"),
        }
    }

    pub fn notebook_shown(&self) {
        self.showing_notebook.store(true, Ordering::SeqCst);
        self.request(Request::Cancel);
    }

    pub fn show_pc(&self) {
        self.showing_notebook.store(false, Ordering::SeqCst);
        self.switch_to_pc(None);
    }

    fn switch_to_pc(&self, delay: Option<Duration>) {
        let request = self
            .setup
            .lock()
            .unwrap()
            .as_ref()
            .map(|setup| Request::Switch {
                name: setup.name.clone(),
                value: setup.pc_input,
                delay: delay.unwrap_or(setup.delay),
            });
        if let Some(request) = request {
            self.request(request);
        }
    }

    pub fn session_ended(&self) {
        if self.showing_notebook.swap(false, Ordering::SeqCst) {
            self.switch_to_pc(Some(Duration::ZERO));
        } else {
            self.request(Request::Flush);
        }
        *self.setup.lock().unwrap() = None;
    }
}
