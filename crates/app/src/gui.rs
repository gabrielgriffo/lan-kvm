use std::error::Error;
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use std::{fs, io};

use eframe::egui;
use lan_kvm_core::config::Config;
use lan_kvm_core::ddc::{input_name, same_monitor, STANDARD_INPUTS};
use lan_kvm_core::pairing::is_valid_code;
use lan_kvm_core::switch::Side;
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use winit::event_loop::{DeviceEvents, EventLoop};

use crate::audio_win;
use crate::autostart;
use crate::ddc_win;
use crate::monitor::{self, Choice, TapChoice};
use crate::net::with_port;
use crate::win::{self, Status, Target};

const POLL: Duration = Duration::from_millis(500);
const WIDTH: f32 = 420.0;
const CONTENT_GAP: f32 = 8.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tone {
    Off,
    Waiting,
    Online,
    Active,
}

fn tone(status: &Status) -> Tone {
    match status {
        Status::Disabled => Tone::Off,
        Status::Connecting { .. } | Status::NotPaired { .. } => Tone::Waiting,
        Status::Connected { .. } => Tone::Online,
        Status::Remote { .. } | Status::Controlled { .. } => Tone::Active,
    }
}

fn tone_rgb(tone: Tone) -> [u8; 3] {
    match tone {
        Tone::Off => [0x8a, 0x8a, 0x8a],
        Tone::Waiting => [0xe0, 0x9a, 0x1a],
        Tone::Online => [0x2e, 0x9d, 0x4f],
        Tone::Active => [0x22, 0xc3, 0xb5],
    }
}

fn status_text(status: &Status) -> String {
    match status {
        Status::Disabled => "Desativado".to_string(),
        Status::Connecting { addr, error: None } => format!("Conectando a {addr}…"),
        Status::Connecting {
            addr,
            error: Some(e),
        } => format!("Sem conexão com {addr}: {e}"),
        Status::Connected {
            addr,
            width,
            height,
        } => format!("Conectado a {addr} (tela remota {width}x{height})"),
        Status::Remote { addr } => format!("Controle no notebook ({addr})"),
        Status::Controlled { addr } => format!("Notebook controlando o PC ({addr})"),
        Status::NotPaired { addr } => format!("Este PC ainda não está pareado com {addr}"),
    }
}

macro_rules! tray_pngs {
    ($state:literal) => {
        [
            (
                16,
                include_bytes!(concat!("../../../branding/tray/tray-", $state, "-16.png"))
                    .as_slice(),
            ),
            (
                20,
                include_bytes!(concat!("../../../branding/tray/tray-", $state, "-20.png"))
                    .as_slice(),
            ),
            (
                24,
                include_bytes!(concat!("../../../branding/tray/tray-", $state, "-24.png"))
                    .as_slice(),
            ),
            (
                32,
                include_bytes!(concat!("../../../branding/tray/tray-", $state, "-32.png"))
                    .as_slice(),
            ),
        ]
    };
}

const WINDOW_ICON: &[u8] = include_bytes!("../../../branding/tray/tray-em-uso-64.png");

fn tray_pngs(tone: Tone) -> [(u32, &'static [u8]); 4] {
    match tone {
        Tone::Off => tray_pngs!("desativado"),
        Tone::Waiting => tray_pngs!("esperando"),
        Tone::Online => tray_pngs!("conectado"),
        Tone::Active => tray_pngs!("em-uso"),
    }
}

fn pick_size(pngs: &[(u32, &'static [u8])], wanted: u32) -> &'static [u8] {
    pngs.iter()
        .find(|(size, _)| *size >= wanted)
        .or(pngs.last())
        .map_or(&[], |(_, bytes)| *bytes)
}

fn decode_png(bytes: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    let mut decoder = png::Decoder::new(io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::ALPHA | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    buf.truncate(info.buffer_size());
    Some((buf, info.width, info.height))
}

fn tray_icon_size() -> u32 {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSMICON};
    let size = unsafe { GetSystemMetrics(SM_CXSMICON) };
    if size > 0 {
        size as u32
    } else {
        16
    }
}

fn tray_icon_for(tone: Tone) -> Option<Icon> {
    let (rgba, w, h) = decode_png(pick_size(&tray_pngs(tone), tray_icon_size()))?;
    Icon::from_rgba(rgba, w, h).ok()
}

fn target_of(config: &Config) -> Option<Target> {
    let addr = config.address.trim();
    (config.enabled && !addr.is_empty()).then(|| Target {
        addr: with_port(addr),
        side: config.side,
        edges: config.edges,
    })
}

fn position_of(config: &Config) -> Option<Side> {
    config.edges.then_some(config.side)
}

fn config_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|dir| PathBuf::from(dir).join("lan-kvm").join("config.txt"))
}

fn load_config() -> Config {
    config_path()
        .and_then(|p| fs::read_to_string(p).ok())
        .map(|text| Config::parse(&text))
        .unwrap_or_default()
}

fn save_config(config: &Config) -> io::Result<()> {
    let path = config_path().ok_or_else(|| io::Error::other("APPDATA não definido"))?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, config.serialize())
}

enum TrayAction {
    Show,
    Toggle,
    Quit,
}

const SHOW_EVENT: &str = "Local\\lan-kvm-show";

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

pub fn show_running() {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenEventW, SetEvent, EVENT_MODIFY_STATE};
    use windows_sys::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, ASFW_ANY};
    let name = wide(SHOW_EVENT);
    unsafe {
        AllowSetForegroundWindow(ASFW_ANY);
        let event = OpenEventW(EVENT_MODIFY_STATE, 0, name.as_ptr());
        if event.is_null() {
            log_warn!("o Lan Kvm já está aberto, mas não respondeu ao pedido para aparecer");
            return;
        }
        SetEvent(event);
        CloseHandle(event);
    }
    thread::sleep(Duration::from_millis(500));
}

fn watch_show_requests(tx: Sender<TrayAction>, ctx: egui::Context) {
    use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
    use windows_sys::Win32::System::Threading::{CreateEventW, WaitForSingleObject, INFINITE};
    let name = wide(SHOW_EVENT);
    let event = unsafe { CreateEventW(std::ptr::null(), 0, 0, name.as_ptr()) };
    if event.is_null() {
        log_warn!("não foi possível criar o aviso para mostrar a janela já aberta");
        return;
    }
    let event = event as isize;
    let _ = thread::Builder::new()
        .name("show".to_string())
        .spawn(move || {
            while unsafe { WaitForSingleObject(event as _, INFINITE) } == WAIT_OBJECT_0 {
                if tx.send(TrayAction::Show).is_err() {
                    return;
                }
                ctx.request_repaint();
            }
        });
}

struct Tray {
    icon: TrayIcon,
    toggle: MenuItem,
    tone: Option<Tone>,
    tooltip: String,
}

impl Tray {
    fn new(ctx: &egui::Context) -> Result<(Self, Receiver<TrayAction>), Box<dyn Error>> {
        let open = MenuItem::new("Abrir Lan Kvm", true, None);
        let toggle = MenuItem::new("Desativar", true, None);
        let quit = MenuItem::new("Sair", true, None);
        let menu = Menu::new();
        menu.append_items(&[&open, &toggle, &PredefinedMenuItem::separator(), &quit])?;
        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_tooltip("Lan Kvm")
            .with_icon(tray_icon_for(Tone::Off).ok_or("ícone inválido")?)
            .build()?;
        let ids: [MenuId; 3] = [open.id().clone(), toggle.id().clone(), quit.id().clone()];

        let (tx, rx) = mpsc::channel();
        let menu_tx = tx.clone();
        let menu_ids = ids;
        let menu_ctx = ctx.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let action = match event.id {
                ref id if *id == menu_ids[0] => TrayAction::Show,
                ref id if *id == menu_ids[1] => TrayAction::Toggle,
                ref id if *id == menu_ids[2] => TrayAction::Quit,
                _ => return,
            };
            let _ = menu_tx.send(action);
            menu_ctx.request_repaint();
        }));
        watch_show_requests(tx.clone(), ctx.clone());
        let click_ctx = ctx.clone();
        TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let _ = tx.send(TrayAction::Show);
                click_ctx.request_repaint();
            }
        }));

        Ok((
            Self {
                icon,
                toggle,
                tone: None,
                tooltip: String::new(),
            },
            rx,
        ))
    }

    fn refresh(&mut self, status: &Status, enabled: bool) {
        let tone = tone(status);
        if self.tone != Some(tone) {
            let _ = self.icon.set_icon(tray_icon_for(tone));
            self.tone = Some(tone);
        }
        let tooltip = format!("Lan Kvm: {}", status_text(status));
        if tooltip != self.tooltip {
            let short: String = tooltip.chars().take(120).collect();
            let _ = self.icon.set_tooltip(Some(short));
            self.tooltip = tooltip;
        }
        self.toggle
            .set_text(if enabled { "Desativar" } else { "Ativar" });
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Pairing {
    Idle,
    Requesting,
    AwaitingCode,
    Verifying,
    Failed(String),
    Done,
}

#[derive(Default)]
struct PairingShared {
    state: Option<Pairing>,
    stream: Option<TcpStream>,
}

fn choice_of(config: &Config) -> Choice {
    Choice {
        enabled: config.monitor_switch,
        name: config.monitor.clone(),
        pc_input: config.pc_input,
        notebook_input: config.notebook_input,
        guard: config.monitor_guard,
    }
}

fn tap_choice_of(config: &Config) -> TapChoice {
    TapChoice {
        enabled: config.tap_switch,
        name: config.tap_monitor.clone(),
        pc_input: config.tap_pc_input,
        notebook_input: config.tap_notebook_input,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    WithControl,
    DoubleTap,
}

type Monitors = Arc<Mutex<Option<Vec<ddc_win::Monitor>>>>;

struct App {
    monitors: Monitors,
    detecting: Arc<std::sync::atomic::AtomicBool>,
    pairing: Arc<Mutex<PairingShared>>,
    code: String,
    config: Config,
    address: String,
    position: Option<Side>,
    autostart: bool,
    error: Option<String>,
    tray: Option<Tray>,
    actions: Option<Receiver<TrayAction>>,
    quitting: bool,
    hide_frames: u8,
}

impl App {
    fn new(ctx: &egui::Context, config: Config, start_hidden: bool) -> Self {
        let (tray, actions, error) = match Tray::new(ctx) {
            Ok((tray, rx)) => (Some(tray), Some(rx), None),
            Err(e) => (
                None,
                None,
                Some(format!("ícone da bandeja indisponível: {e}")),
            ),
        };
        Self {
            monitors: Arc::default(),
            detecting: Arc::default(),
            pairing: Arc::default(),
            code: String::new(),
            address: config.address.clone(),
            position: position_of(&config),
            config,
            autostart: autostart::is_enabled(),
            error,
            tray,
            actions,
            quitting: false,
            hide_frames: if start_hidden { 3 } else { 0 },
        }
    }

    fn commit(&mut self) {
        win::set_target(target_of(&self.config));
        self.error = save_config(&self.config)
            .err()
            .map(|e| format!("não foi possível salvar a configuração: {e}"));
        if let Some(e) = &self.error {
            log_error!("{e}");
        }
    }

    fn detect_monitors(&self, ctx: &egui::Context) {
        use std::sync::atomic::Ordering;
        if self.detecting.swap(true, Ordering::SeqCst) {
            return;
        }
        let (monitors, detecting, ctx) = (
            Arc::clone(&self.monitors),
            Arc::clone(&self.detecting),
            ctx.clone(),
        );
        thread::spawn(move || {
            let list = ddc_win::list();
            log_info!(
                "monitores detectados por DDC: {}",
                list.iter()
                    .map(|m| m.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            *monitors.lock().unwrap() = Some(list);
            detecting.store(false, Ordering::SeqCst);
            ctx.request_repaint();
        });
    }

    fn apply_monitor(&mut self) {
        win::set_monitor(choice_of(&self.config));
        win::set_tap(tap_choice_of(&self.config));
        self.commit();
    }

    fn fields(&mut self, section: Section) -> (&mut String, &mut u8, &mut u8) {
        let c = &mut self.config;
        match section {
            Section::WithControl => (&mut c.monitor, &mut c.pc_input, &mut c.notebook_input),
            Section::DoubleTap => (
                &mut c.tap_monitor,
                &mut c.tap_pc_input,
                &mut c.tap_notebook_input,
            ),
        }
    }

    fn monitor_ui(&mut self, ui: &mut egui::Ui, section: Section) {
        let detecting = self.detecting.load(std::sync::atomic::Ordering::SeqCst);
        let list = self.monitors.lock().unwrap().clone();
        let chosen = self.fields(section).0.clone();
        let selected = list
            .as_ref()
            .and_then(|l| l.iter().find(|m| same_monitor(&m.name, &chosen)).cloned());
        let salt = match section {
            Section::WithControl => "controle",
            Section::DoubleTap => "batidas",
        };
        let mut detect = false;
        let inputs: Vec<u8> = selected
            .as_ref()
            .map(|m| m.inputs.clone())
            .filter(|i| !i.is_empty())
            .unwrap_or_else(|| STANDARD_INPUTS.to_vec());
        let current = selected.as_ref().and_then(|m| m.current);
        let label = |v: u8| {
            if v == 0 {
                "escolha".to_string()
            } else if Some(v) == current {
                format!("{} (0x{v:02X}, atual)", input_name(v))
            } else {
                format!("{} (0x{v:02X})", input_name(v))
            }
        };
        let mut changed = false;
        let (name, pc_input, notebook_input) = self.fields(section);
        egui::Grid::new(("monitor", salt))
            .num_columns(2)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                ui.label("Monitor");
                ui.horizontal(|ui| {
                    let shown = if name.is_empty() {
                        "escolha".to_string()
                    } else {
                        name.clone()
                    };
                    egui::ComboBox::from_id_salt(("monitor-name", salt))
                        .selected_text(shown)
                        .show_ui(ui, |ui| {
                            for m in list.iter().flatten() {
                                changed |= ui
                                    .selectable_value(&mut *name, m.name.clone(), &m.name)
                                    .changed();
                            }
                        });
                    if detecting {
                        ui.spinner();
                    } else if ui.button("Detectar").clicked() {
                        detect = true;
                    }
                });
                ui.end_row();
                for (title, value) in [
                    ("Entrada do PC", pc_input),
                    ("Entrada do notebook", notebook_input),
                ] {
                    ui.label(title);
                    egui::ComboBox::from_id_salt((title, salt))
                        .selected_text(label(*value))
                        .show_ui(ui, |ui| {
                            for &v in &inputs {
                                changed |= ui.selectable_value(value, v, label(v)).changed();
                            }
                        });
                    ui.end_row();
                }
            });
        if detect {
            self.detect_monitors(ui.ctx());
        }
        if section == Section::DoubleTap {
            if changed {
                self.apply_monitor();
            }
            return;
        }
        changed |= ui
            .checkbox(
                &mut self.config.monitor_guard,
                "Evitar troca acidental do monitor",
            )
            .on_hover_text(
                "A entrada só muda depois de 3s seguidos na outra tela. \
                 Voltar antes disso zera a contagem.",
            )
            .changed();
        if changed {
            self.apply_monitor();
        }
        let red = egui::Color32::from_rgb(0xc0, 0x30, 0x30);
        for (who, report) in [
            ("PC", monitor::pc_report()),
            ("Notebook", monitor::notebook_report()),
        ] {
            match report {
                Some((true, text)) => {
                    ui.weak(format!("{who}: {text}"));
                }
                Some((false, text)) => {
                    ui.colored_label(red, format!("{who}: {text}"));
                }
                None => {}
            }
        }
    }

    fn pairing_state(&self) -> Pairing {
        self.pairing
            .lock()
            .unwrap()
            .state
            .clone()
            .unwrap_or(Pairing::Idle)
    }

    fn set_pairing(shared: &Mutex<PairingShared>, state: Pairing, stream: Option<TcpStream>) {
        let mut shared = shared.lock().unwrap();
        shared.state = Some(state);
        shared.stream = stream;
    }

    fn start_pairing(&mut self, ctx: &egui::Context) {
        let addr = with_port(self.config.address.trim());
        let shared = Arc::clone(&self.pairing);
        let ctx = ctx.clone();
        self.code.clear();
        Self::set_pairing(&shared, Pairing::Requesting, None);
        thread::spawn(move || {
            match win::pair_request(&addr) {
                Ok(stream) => Self::set_pairing(&shared, Pairing::AwaitingCode, Some(stream)),
                Err(e) => Self::set_pairing(&shared, Pairing::Failed(e.to_string()), None),
            }
            ctx.request_repaint();
        });
    }

    fn submit_code(&mut self, ctx: &egui::Context) {
        let stream = self.pairing.lock().unwrap().stream.take();
        let Some(stream) = stream else {
            return;
        };
        let code = self.code.trim().to_string();
        let shared = Arc::clone(&self.pairing);
        let ctx = ctx.clone();
        Self::set_pairing(&shared, Pairing::Verifying, None);
        thread::spawn(move || {
            let next = match win::pair_complete(stream, &code) {
                Ok(()) => Pairing::Done,
                Err(e) => Pairing::Failed(e.to_string()),
            };
            Self::set_pairing(&shared, next, None);
            ctx.request_repaint();
        });
    }

    fn pairing_ui(&mut self, ui: &mut egui::Ui, status: &Status) -> bool {
        let state = self.pairing_state();
        let unpaired = matches!(status, Status::NotPaired { .. });
        let shown = unpaired || !matches!(state, Pairing::Idle);
        let red = egui::Color32::from_rgb(0xc0, 0x30, 0x30);
        match state {
            Pairing::Requesting => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Pedindo um código ao notebook…");
                });
            }
            Pairing::AwaitingCode => {
                ui.label("Digite o código que apareceu no notebook:");
                ui.horizontal(|ui| {
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut self.code)
                            .char_limit(6)
                            .desired_width(80.0)
                            .hint_text("000000"),
                    );
                    let ready = is_valid_code(self.code.trim());
                    let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if ui
                        .add_enabled(ready, egui::Button::new("Confirmar"))
                        .clicked()
                        || (ready && enter)
                    {
                        self.submit_code(ui.ctx());
                    }
                    if ui.button("Cancelar").clicked() {
                        Self::set_pairing(&self.pairing, Pairing::Idle, None);
                    }
                });
            }
            Pairing::Verifying => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Verificando o código…");
                });
            }
            Pairing::Failed(e) => {
                ui.colored_label(red, format!("Não foi possível parear: {e}"));
                if ui.button("Tentar de novo").clicked() {
                    self.start_pairing(ui.ctx());
                }
            }
            Pairing::Done if !unpaired => {
                ui.label("Pareado com sucesso.");
            }
            Pairing::Idle | Pairing::Done => {
                if unpaired {
                    let can = !self.config.address.trim().is_empty();
                    if ui
                        .add_enabled(can, egui::Button::new("Parear com o notebook"))
                        .clicked()
                    {
                        self.start_pairing(ui.ctx());
                    }
                }
            }
        }
        shown
    }

    fn handle_tray(&mut self, ctx: &egui::Context) {
        let actions: Vec<TrayAction> = match &self.actions {
            Some(rx) => rx.try_iter().collect(),
            None => Vec::new(),
        };
        for action in actions {
            match action {
                TrayAction::Show => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                TrayAction::Toggle => {
                    self.config.enabled = !self.config.enabled;
                    self.commit();
                }
                TrayAction::Quit => {
                    log_info!("saindo pelo menu da bandeja");
                    self.quitting = true;
                    win::set_target(None);
                    self.tray = None;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.hide_frames > 0 {
            self.hide_frames -= 1;
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            ctx.request_repaint();
        }
        self.handle_tray(ctx);
        if ctx.input(|i| i.viewport().close_requested()) && !self.quitting {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
        let status = win::status();
        if let Some(tray) = self.tray.as_mut() {
            tray.refresh(&status, self.config.enabled);
        }
        ctx.request_repaint_after(POLL);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let status = win::status();
        let footer = egui::Panel::bottom("rodape")
            .show(ui, |ui| {
                ui.add_space(4.0);
                ui.weak("Ctrl+Alt+Backspace devolve o controle ao Windows.");
                ui.weak("Fechar a janela mantém o Lan Kvm na bandeja.");
                ui.weak(format!("Versão: {}", lan_kvm_core::VERSION));
                ui.separator();
                ui.horizontal(|ui| {
                    let [r, g, b] = tone_rgb(tone(&status));
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                    ui.painter().circle_filled(
                        rect.center(),
                        6.0,
                        egui::Color32::from_rgb(r, g, b),
                    );
                    ui.add(egui::Label::new(status_text(&status)).wrap());
                });
                ui.add_space(4.0);
            })
            .response
            .rect
            .height();
        let mut content_bottom = 0.0;
        egui::CentralPanel::default().show(ui, |ui| {
            if self.pairing_ui(ui, &status) {
                ui.separator();
            }

            egui::Grid::new("config")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Notebook");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.address)
                            .hint_text("IP ou nome, ex.: 192.168.0.18"),
                    );
                    ui.end_row();
                    ui.label("Posição");
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut self.position, Some(Side::Left), "À esquerda");
                        ui.radio_value(&mut self.position, Some(Side::Right), "À direita");
                        ui.radio_value(&mut self.position, None, "Desativado")
                            .on_hover_text(
                                "O controle não passa pela borda da tela; use as duas batidas \
                                 no topo para trocar.",
                            );
                    });
                    ui.end_row();
                });

            let changed = self.address.trim() != self.config.address.trim()
                || self.position != position_of(&self.config);
            ui.add_space(4.0);
            let paired = !matches!(status, Status::NotPaired { .. } | Status::Disabled);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(changed, egui::Button::new("Salvar"))
                    .clicked()
                {
                    self.config.address = self.address.trim().to_string();
                    self.config.edges = self.position.is_some();
                    if let Some(side) = self.position {
                        self.config.side = side;
                    }
                    self.commit();
                }
                if paired && ui.button("Esquecer pareamento").clicked() {
                    self.error = win::forget_pairing()
                        .err()
                        .map(|e| format!("não foi possível esquecer o pareamento: {e}"));
                    Self::set_pairing(&self.pairing, Pairing::Idle, None);
                }
            });
            ui.separator();

            if ui.checkbox(&mut self.config.enabled, "Ativado").changed() {
                self.commit();
            }
            if ui
                .checkbox(
                    &mut self.config.protect_corners,
                    "Proteger os cantos da borda",
                )
                .on_hover_text(
                    "Os 5% de cima e os 5% de baixo da borda do notebook não trocam de tela.",
                )
                .changed()
            {
                win::set_corner_guard(self.config.protect_corners);
                self.commit();
            }
            if ui
                .checkbox(
                    &mut self.config.monitor_switch,
                    "Trocar a entrada do monitor junto com o controle",
                )
                .on_hover_text(
                    "O monitor só aceita comandos da entrada que está mostrando: o PC troca \
                     para a entrada do notebook e o notebook troca de volta para a do PC. \
                     Os nomes seguem o padrão DDC/CI, mas alguns monitores numeram as \
                     entradas de outro jeito: vale o código que funcionar.",
                )
                .changed()
            {
                if self.config.monitor_switch && self.monitors.lock().unwrap().is_none() {
                    self.detect_monitors(ui.ctx());
                }
                self.apply_monitor();
            }
            if self.config.monitor_switch {
                ui.indent("monitor-section", |ui| {
                    self.monitor_ui(ui, Section::WithControl)
                });
            }
            if ui
                .checkbox(
                    &mut self.config.tap_switch,
                    "Trocar a entrada do monitor com duas batidas no topo",
                )
                .on_hover_text(
                    "Bata o mouse duas vezes na borda de cima da tela, em menos de 1 s, \
                     para alternar o monitor entre a entrada do PC e a do notebook. \
                     O mouse e o teclado vão junto para o sistema que aparecer na tela.",
                )
                .changed()
            {
                if self.config.tap_switch && self.monitors.lock().unwrap().is_none() {
                    self.detect_monitors(ui.ctx());
                }
                self.apply_monitor();
            }
            if self.config.tap_switch {
                ui.indent("tap-section", |ui| self.monitor_ui(ui, Section::DoubleTap));
            }
            if ui
                .checkbox(
                    &mut self.config.audio,
                    "Usar o fone e o microfone do PC no notebook",
                )
                .on_hover_text(
                    "O som do notebook toca no fone do PC e o microfone do PC vira o \
                     microfone do notebook, enquanto estiverem conectados.",
                )
                .changed()
            {
                win::set_audio(self.config.audio);
                self.commit();
            }
            if self.config.audio {
                ui.indent("audio-section", |ui| {
                    let red = egui::Color32::from_rgb(0xc0, 0x30, 0x30);
                    for (who, report) in [
                        ("PC", audio_win::pc_report()),
                        ("Notebook", audio_win::notebook_report()),
                    ] {
                        match report {
                            Some((true, text)) => {
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(format!("{who}: {text}")).weak(),
                                    )
                                    .wrap(),
                                );
                            }
                            Some((false, text)) => {
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(format!("{who}: {text}")).color(red),
                                    )
                                    .wrap(),
                                );
                            }
                            None => {}
                        }
                    }
                });
            }
            if ui
                .checkbox(&mut self.autostart, "Iniciar com o Windows")
                .changed()
            {
                match autostart::set_enabled(self.autostart) {
                    Ok(()) => log_info!("iniciar com o Windows: {}", self.autostart),
                    Err(e) => {
                        let e = format!("não foi possível alterar o início automático: {e}");
                        log_error!("{e}");
                        self.error = Some(e);
                        self.autostart = autostart::is_enabled();
                    }
                }
            }
            if let Some(e) = &self.error {
                ui.add_space(4.0);
                ui.colored_label(egui::Color32::from_rgb(0xc0, 0x30, 0x30), e);
            }
            content_bottom = ui.cursor().min.y;
        });
        let wanted = (content_bottom + CONTENT_GAP + footer).round();
        let current = ui
            .ctx()
            .input(|i| i.viewport().inner_rect.map(|r| r.height()));
        if current.is_some_and(|h| (h - wanted).abs() > 1.0) {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(WIDTH, wanted)));
        }
    }
}

pub fn run(start_hidden: bool) -> Result<(), Box<dyn Error>> {
    win::start()?;
    let config = load_config();
    win::set_corner_guard(config.protect_corners);
    win::set_monitor(choice_of(&config));
    win::set_tap(tap_choice_of(&config));
    win::set_audio(config.audio);
    win::set_target(target_of(&config));

    let (rgba, icon_w, icon_h) = decode_png(WINDOW_ICON).ok_or("ícone da janela inválido")?;
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Lan Kvm")
            .with_inner_size([WIDTH, 515.0])
            .with_resizable(false)
            .with_maximize_button(false)
            .with_visible(!start_hidden)
            .with_icon(egui::IconData {
                rgba,
                width: icon_w,
                height: icon_h,
            }),
        ..Default::default()
    };
    let event_loop = EventLoop::<eframe::UserEvent>::with_user_event().build()?;
    event_loop.listen_device_events(DeviceEvents::Never);
    let mut app = eframe::create_native(
        "lan-kvm",
        options,
        Box::new(move |cc| {
            win::reclaim_raw_input();
            cc.egui_ctx
                .all_styles_mut(|style| style.spacing.tooltip_width = 300.0);
            let app = App::new(&cc.egui_ctx, config, start_hidden);
            if app.config.monitor_switch || app.config.tap_switch {
                app.detect_monitors(&cc.egui_ctx);
            }
            Ok(Box::new(app))
        }),
        &event_loop,
    );
    event_loop.run_app(&mut app)?;
    win::set_target(None);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_requires_enabled_and_address() {
        let mut config = Config {
            address: " 192.168.0.18 ".to_string(),
            side: Side::Left,
            enabled: true,
            protect_corners: false,
            ..Config::default()
        };
        assert_eq!(
            target_of(&config),
            Some(Target {
                addr: "192.168.0.18:24801".to_string(),
                side: Side::Left,
                edges: true,
            })
        );
        config.enabled = false;
        assert_eq!(target_of(&config), None);
        config.enabled = true;
        config.address = "   ".to_string();
        assert_eq!(target_of(&config), None);
    }

    #[test]
    fn tone_follows_status() {
        let addr = "x".to_string();
        assert_eq!(tone(&Status::Disabled), Tone::Off);
        assert_eq!(
            tone(&Status::Connecting {
                addr: addr.clone(),
                error: None
            }),
            Tone::Waiting
        );
        assert_eq!(
            tone(&Status::Connected {
                addr: addr.clone(),
                width: 1,
                height: 1
            }),
            Tone::Online
        );
        assert_eq!(tone(&Status::Remote { addr: addr.clone() }), Tone::Active);
        assert_eq!(
            tone(&Status::Controlled { addr: addr.clone() }),
            Tone::Active
        );
        assert_eq!(tone(&Status::NotPaired { addr }), Tone::Waiting);
    }

    #[test]
    fn every_tone_has_a_tray_icon_for_each_size() {
        for tone in [Tone::Off, Tone::Waiting, Tone::Online, Tone::Active] {
            for (size, bytes) in tray_pngs(tone) {
                let (rgba, w, h) = decode_png(bytes).expect("png válido");
                assert_eq!((w, h), (size, size));
                assert_eq!(rgba.len(), (size * size * 4) as usize);
            }
        }
        let (_, w, h) = decode_png(WINDOW_ICON).unwrap();
        assert_eq!((w, h), (64, 64));
    }

    #[test]
    fn tray_size_follows_the_system_small_icon() {
        let pngs = tray_pngs(Tone::Online);
        let size_of = |bytes: &[u8]| decode_png(bytes).unwrap().1;
        assert_eq!(size_of(pick_size(&pngs, 16)), 16);
        assert_eq!(size_of(pick_size(&pngs, 20)), 20);
        assert_eq!(size_of(pick_size(&pngs, 22)), 24);
        assert_eq!(size_of(pick_size(&pngs, 40)), 32);
    }

    #[test]
    fn tray_icons_differ_by_state() {
        let pixels = |tone| decode_png(tray_pngs(tone)[3].1).unwrap().0;
        let all = [Tone::Off, Tone::Waiting, Tone::Online, Tone::Active].map(pixels);
        for i in 0..all.len() {
            for j in i + 1..all.len() {
                assert_ne!(all[i], all[j]);
            }
        }
    }
}
