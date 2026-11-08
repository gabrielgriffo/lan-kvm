use std::error::Error;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};
use std::{mem, ptr, thread};

use lan_kvm_core::ballistics::{Ballistics, Coalescing};
use lan_kvm_core::clipboard::{to_windows, ClipSync, MAX_TEXT};
use lan_kvm_core::ddc::same_monitor;
use lan_kvm_core::keymap::{evdev_from_windows_scancode, KEY_PAUSE};
use lan_kvm_core::keys::{KeyRouter, Route};
use lan_kvm_core::pairing::{pair_initiator, reply_text, MODE_PAIR, MODE_SESSION, REPLY_OK};
use lan_kvm_core::proto::{Button, Message, PROTOCOL_VERSION};
use lan_kvm_core::puppet::{Puppet, PuppetMove};
use lan_kvm_core::secure::{handshake_initiator, Key, SecureReader, SecureWriter};
use lan_kvm_core::switch::{Action, Rect, Side, Switcher};
use lan_kvm_core::tap::{arrival, restful, top_distance, LastSpot, TopTaps};
use windows_sys::Win32::Foundation::{HMODULE, HWND};
use windows_sys::Win32::Foundation::{LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
};
use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, VK_BACK, VK_CONTROL, VK_MENU, VK_PAUSE,
};
use windows_sys::Win32::UI::Input::{
    GetRawInputData, RegisterRawInputDevices, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER,
    RIDEV_INPUTSINK, RID_INPUT, RIM_TYPEMOUSE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::audio_win;
use crate::inject::{self, Injection};
use crate::keystore;
use crate::monitor::{self, Choice};
use crate::mouse_speed;
use crate::net::{write_msg, Deadline, MsgReader, PEER_TIMEOUT};
use crate::pointer;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const RETRY_INTERVAL: Duration = Duration::from_secs(2);
const PING_INTERVAL: Duration = Duration::from_secs(1);
const PAIR_TIMEOUT: Duration = Duration::from_secs(150);
const CLIPBOARD_WINDOW: Duration = Duration::from_secs(2);
const MAX_DELTA: i32 = 2048;
const GIVE_BACK_MIDDLE: u16 = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub addr: String,
    pub side: Side,
    pub edges: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Disabled,
    Connecting {
        addr: String,
        error: Option<String>,
    },
    Connected {
        addr: String,
        width: u16,
        height: u16,
    },
    Remote {
        addr: String,
    },
    Controlled {
        addr: String,
    },
    NotPaired {
        addr: String,
    },
}

struct Control {
    target: Option<Target>,
    generation: u64,
    status: Status,
}

static CONTROL: Mutex<Control> = Mutex::new(Control {
    target: None,
    generation: 0,
    status: Status::Disabled,
});
static WAKE: Condvar = Condvar::new();
static STARTED: OnceLock<Result<(), String>> = OnceLock::new();

struct Link {
    switcher: Switcher,
    ballistics: Ballistics,
    reports: Coalescing,
    taps: TopTaps,
    pc_spot: LastSpot,
    remote_spot: LastSpot,
    remote_screens: Vec<(Rect, String)>,
    tx: Sender<Message>,
    monitors: Vec<Rect>,
    side: Side,
    preferred: usize,
    controlled: Option<Controlled>,
}

impl Link {
    fn watch_top(&mut self, distance: Option<i32>) {
        if self.taps.update(distance, Instant::now()) && monitor::tap_enabled() {
            monitor::tap_toggle(self.tx.clone(), follow_monitor);
        }
    }
}

struct Controlled {
    puppet: Puppet,
    keys: Vec<u16>,
    buttons: Vec<Button>,
}

impl Controlled {
    fn release(self) -> Vec<Injection> {
        let keys = self.keys.into_iter().map(|k| Injection::Key(k, false));
        let buttons = self
            .buttons
            .into_iter()
            .map(|b| Injection::Button(b, false));
        keys.chain(buttons).collect()
    }
}

struct Ctx {
    keys: KeyRouter,
    link: Option<Link>,
}

static CTX: Mutex<Option<Ctx>> = Mutex::new(None);
static CORNER_GUARD: AtomicBool = AtomicBool::new(false);
static AUDIO: AtomicBool = AtomicBool::new(false);
static RAW_WINDOW: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
const WM_REGISTER_RAW: u32 = WM_APP + 1;

pub fn set_monitor(choice: Choice) {
    if choice.enabled {
        log_info!(
            "troca de monitor: {} (PC 0x{:02X}, notebook 0x{:02X}, {})",
            choice.name,
            choice.pc_input,
            choice.notebook_input,
            if choice.guard {
                "espera 3 s"
            } else {
                "imediata"
            }
        );
    } else {
        log_info!("troca de monitor desligada");
    }
    monitor::set(choice);
    if let Some(link) = CTX
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|ctx| ctx.link.as_ref())
    {
        let _ = link.tx.send(monitor::setup_message());
    }
}

pub fn set_tap(choice: monitor::TapChoice) {
    monitor::set_tap(choice);
    if let Some(link) = CTX
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|ctx| ctx.link.as_ref())
    {
        let _ = link.tx.send(monitor::tap_message());
    }
}

pub fn set_audio(on: bool) {
    if AUDIO.swap(on, Ordering::SeqCst) == on {
        return;
    }
    log_info!(
        "fone e microfone do PC no notebook: {}",
        if on { "ligado" } else { "desligado" }
    );
    audio_win::clear_reports();
    let tx = CTX
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|ctx| ctx.link.as_ref())
        .map(|link| link.tx.clone());
    let Some(tx) = tx else {
        return;
    };
    let _ = tx.send(Message::AudioSetup { enabled: on });
    if on {
        audio_win::start(tx);
    } else {
        audio_win::stop();
    }
}

pub fn set_corner_guard(on: bool) {
    CORNER_GUARD.store(on, Ordering::SeqCst);
    if let Some(link) = CTX
        .lock()
        .unwrap()
        .as_mut()
        .and_then(|ctx| ctx.link.as_mut())
    {
        link.switcher.set_corner_guard(on);
    }
    log_info!(
        "proteção dos cantos da borda: {}",
        if on { "ligada" } else { "desligada" }
    );
}

pub fn start() -> Result<(), Box<dyn Error>> {
    STARTED
        .get_or_init(|| {
            unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
            pointer::undo_old_hide();
            *CTX.lock().unwrap() = Some(Ctx {
                keys: KeyRouter::default(),
                link: None,
            });
            let (ready_tx, ready_rx) = mpsc::channel();
            thread::spawn(move || run_hooks(ready_tx));
            let ready = ready_rx
                .recv()
                .unwrap_or_else(|_| Err("thread dos hooks encerrou".to_string()));
            if ready.is_ok() {
                thread::spawn(connection_loop);
            }
            ready
        })
        .clone()
        .map_err(Into::into)
}

pub fn set_target(target: Option<Target>) {
    {
        let mut control = CONTROL.lock().unwrap();
        if control.target == target {
            return;
        }
        control.status = match &target {
            Some(t) => Status::Connecting {
                addr: t.addr.clone(),
                error: None,
            },
            None => Status::Disabled,
        };
        match &target {
            Some(t) if t.edges => log_info!("ativado: notebook {} ({:?})", t.addr, t.side),
            Some(t) => log_info!("ativado: notebook {} (sem troca pela borda)", t.addr),
            None => log_info!("desativado"),
        }
        control.target = target;
        control.generation += 1;
    }
    drop_link();
    WAKE.notify_all();
}

pub fn status() -> Status {
    let status = CONTROL.lock().unwrap().status.clone();
    let (remote, controlled) = CTX
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|ctx| ctx.link.as_ref())
        .map_or((false, false), |link| {
            (link.switcher.is_remote(), link.controlled.is_some())
        });
    match status {
        Status::Connected { addr, .. } if remote => Status::Remote { addr },
        Status::Connected { addr, .. } if controlled => Status::Controlled { addr },
        other => other,
    }
}

unsafe extern "system" fn console_ctrl(_: u32) -> i32 {
    pointer::show();
    0
}

pub fn run_sender(addr: &str, side: Side) -> Result<(), Box<dyn Error>> {
    unsafe { SetConsoleCtrlHandler(Some(console_ctrl), 1) };
    start()?;
    set_target(Some(Target {
        addr: addr.to_string(),
        side,
        edges: true,
    }));
    loop {
        thread::park();
    }
}

fn run_hooks(ready: Sender<Result<(), String>>) {
    unsafe {
        let module = GetModuleHandleW(ptr::null());
        let mut hooks = Vec::new();
        for (id, proc_) in [
            (WH_MOUSE_LL, mouse_proc as HookProc),
            (WH_KEYBOARD_LL, keyboard_proc as HookProc),
        ] {
            let hook = SetWindowsHookExW(id, Some(proc_), module, 0);
            if hook.is_null() {
                let _ = ready.send(Err(io::Error::last_os_error().to_string()));
                return;
            }
            hooks.push(hook);
        }
        if let Err(e) = register_raw_input(module) {
            let _ = ready.send(Err(e));
            return;
        }
        let _ = ready.send(Ok(()));
        let mut msg: MSG = mem::zeroed();
        while GetMessageW(&mut msg, ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        for hook in hooks {
            UnhookWindowsHookEx(hook);
        }
    }
}

unsafe fn register_raw_input(module: HMODULE) -> Result<(), String> {
    let class: Vec<u16> = "lan-kvm-raw-input".encode_utf16().chain(Some(0)).collect();
    let wc = WNDCLASSW {
        lpfnWndProc: Some(raw_input_proc),
        hInstance: module,
        lpszClassName: class.as_ptr(),
        ..mem::zeroed()
    };
    RegisterClassW(&wc);
    let window = CreateWindowExW(
        0,
        class.as_ptr(),
        class.as_ptr(),
        0,
        0,
        0,
        0,
        0,
        HWND_MESSAGE,
        ptr::null_mut(),
        module,
        ptr::null(),
    );
    if window.is_null() {
        return Err(format!(
            "janela de Raw Input: {}",
            io::Error::last_os_error()
        ));
    }
    RAW_WINDOW.store(window as isize, Ordering::SeqCst);
    register_mouse(window)
}

unsafe fn register_mouse(window: HWND) -> Result<(), String> {
    let device = RAWINPUTDEVICE {
        usUsagePage: 1,
        usUsage: 2,
        dwFlags: RIDEV_INPUTSINK,
        hwndTarget: window,
    };
    if RegisterRawInputDevices(&device, 1, mem::size_of::<RAWINPUTDEVICE>() as u32) == 0 {
        return Err(format!("Raw Input: {}", io::Error::last_os_error()));
    }
    Ok(())
}

pub fn reclaim_raw_input() {
    let window = RAW_WINDOW.load(Ordering::SeqCst) as HWND;
    if !window.is_null() {
        unsafe { PostMessageW(window, WM_REGISTER_RAW, 0, 0) };
    }
}

unsafe extern "system" fn raw_input_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_REGISTER_RAW {
        match register_mouse(hwnd) {
            Ok(()) => log_info!("Raw Input do mouse registrado de novo após a janela"),
            Err(e) => log_error!("{e}"),
        }
        return 0;
    }
    if msg == WM_INPUT {
        let mut raw: RAWINPUT = mem::zeroed();
        let mut size = mem::size_of::<RAWINPUT>() as u32;
        let read = GetRawInputData(
            lparam as HRAWINPUT,
            RID_INPUT,
            (&mut raw as *mut RAWINPUT).cast(),
            &mut size,
            mem::size_of::<RAWINPUTHEADER>() as u32,
        );
        if read != u32::MAX && raw.header.dwType == RIM_TYPEMOUSE {
            let mouse = raw.data.mouse;
            let absolute = mouse.usFlags & 1 != 0;
            let ours = mouse.ulExtraInformation as usize == inject::MAGIC;
            if !absolute && !ours && (mouse.lLastX != 0 || mouse.lLastY != 0) {
                on_raw_delta(mouse.lLastX, mouse.lLastY);
            }
        }
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

fn on_raw_delta(dx: i32, dy: i32) {
    let mut warp = None;
    {
        let mut guard = CTX.lock().unwrap();
        let Some(link) = guard.as_mut().and_then(|ctx| ctx.link.as_mut()) else {
            return;
        };
        if link.controlled.is_some() {
            return;
        }
        let reports = link.reports.take();
        let (dx, dy) = link.ballistics.apply_batched(dx, dy, reports);
        match link.switcher.on_delta(dx, dy) {
            Action::Motion { x, y } => {
                let _ = link.tx.send(Message::Motion { x, y });
                let now = Instant::now();
                let rest = restful(Some(y as i32), &link.taps, now);
                link.remote_spot.observe((x as i32, y as i32), rest, now);
                link.watch_top(Some(y as i32));
            }
            Action::Leave { x, y } => {
                let _ = link.tx.send(Message::Leave);
                link.taps.reset();
                warp = Some((x, y));
            }
            Action::Block => {
                let y = link.switcher.remote_position().map(|(_, y)| y);
                link.watch_top(y);
            }
            _ => {}
        }
    }
    if let Some((x, y)) = warp {
        return_pointer(x, y);
    }
}

fn follow_monitor(to_notebook: bool) {
    let mut warp = None;
    let mut hide = false;
    {
        let mut guard = CTX.lock().unwrap();
        let Some(link) = guard.as_mut().and_then(|ctx| ctx.link.as_mut()) else {
            return;
        };
        if link.controlled.is_some() {
            return;
        }
        if to_notebook {
            let mut cursor = POINT { x: 0, y: 0 };
            unsafe { GetCursorPos(&mut cursor) };
            let entry = link
                .monitors
                .iter()
                .position(|m| m.contains(cursor.x, cursor.y))
                .unwrap_or(link.preferred);
            let screen = monitor::tap_name()
                .and_then(|name| {
                    link.remote_screens
                        .iter()
                        .find(|(_, n)| same_monitor(n, &name))
                        .map(|(r, _)| *r)
                })
                .unwrap_or_else(|| link.switcher.remote_area());
            let at = arrival(link.remote_spot.get(), screen);
            if let Some(Action::Enter { x, y }) = link.switcher.force_enter_at(entry, at) {
                log_info!("duas batidas no topo: controle passou para o notebook em {x}, {y}");
                link.ballistics = mouse_speed::ballistics();
                link.reports.reset();
                link.taps.reset();
                let _ = link.tx.send(Message::Enter { x, y });
                hide = true;
            }
        } else if let Some(Action::Leave { x, y }) = link.switcher.force_leave() {
            let _ = link.tx.send(Message::Leave);
            link.taps.reset();
            let back = link.monitors.iter().find(|m| m.contains(x, y));
            warp = Some(back.map_or((x, y), |m| arrival(link.pc_spot.get(), *m)));
        }
    }
    if let Some((x, y)) = warp {
        return_pointer(x, y);
    }
    if hide {
        pointer::hide();
    }
}

fn set_remote_screens(screens: Vec<(Rect, String)>) {
    log_info!("telas do notebook: {screens:?}");
    if let Some(link) = CTX
        .lock()
        .unwrap()
        .as_mut()
        .and_then(|ctx| ctx.link.as_mut())
    {
        link.remote_screens = screens;
    }
}

fn return_pointer(x: i32, y: i32) {
    log_info!("controle voltou ao PC em {x}, {y}");
    unsafe { SetCursorPos(x, y) };
    pointer::show();
}

fn set_status(generation: u64, status: Status) {
    let mut control = CONTROL.lock().unwrap();
    if control.generation == generation {
        control.status = status;
    }
}

fn wait_for_target() -> (u64, Target) {
    let mut control = CONTROL.lock().unwrap();
    loop {
        if let Some(target) = control.target.clone() {
            return (control.generation, target);
        }
        control = WAKE.wait(control).unwrap();
    }
}

fn wait_retry(generation: u64) {
    let control = CONTROL.lock().unwrap();
    let _ = WAKE
        .wait_timeout_while(control, RETRY_INTERVAL, |c| c.generation == generation)
        .unwrap();
}

fn connection_loop() {
    let mut last_error = String::new();
    loop {
        let (generation, target) = wait_for_target();
        let addr = target.addr.clone();
        let token = match keystore::load() {
            Ok(Some(token)) => token,
            Ok(None) => {
                set_status(generation, Status::NotPaired { addr });
                wait_retry(generation);
                continue;
            }
            Err(e) => {
                let error = format!("não foi possível ler o pareamento: {e}");
                log_error!("{error}");
                set_status(
                    generation,
                    Status::Connecting {
                        addr,
                        error: Some(error),
                    },
                );
                wait_retry(generation);
                continue;
            }
        };
        match connect(&addr, &token) {
            Ok((stream, reader, writer, size)) => {
                last_error.clear();
                match monitors() {
                    Ok((monitors, park)) => {
                        log_info!(
                            "conectado a {addr}: tela remota {}x{}, monitores locais {monitors:?}",
                            size.0,
                            size.1
                        );
                        set_status(
                            generation,
                            Status::Connected {
                                addr: addr.clone(),
                                width: size.0,
                                height: size.1,
                            },
                        );
                        let preferred = monitors
                            .iter()
                            .position(|m| m.contains(park.0, park.1))
                            .unwrap_or(0);
                        let link = Link {
                            switcher: {
                                let mut switcher =
                                    Switcher::new(monitors.clone(), size, target.side);
                                switcher.set_corner_guard(CORNER_GUARD.load(Ordering::SeqCst));
                                switcher.set_edges(target.edges);
                                switcher
                            },
                            ballistics: mouse_speed::ballistics(),
                            reports: Coalescing::default(),
                            taps: TopTaps::default(),
                            pc_spot: LastSpot::default(),
                            remote_spot: LastSpot::default(),
                            remote_screens: Vec::new(),
                            tx: mpsc::channel().0,
                            monitors,
                            side: target.side,
                            preferred,
                            controlled: None,
                        };
                        let reason = session(stream, reader, writer, link, generation);
                        log_warn!("desconectado: {reason}");
                        set_status(
                            generation,
                            Status::Connecting {
                                addr,
                                error: Some(reason),
                            },
                        );
                    }
                    Err(e) => log_error!("erro ao ler os monitores: {e}"),
                }
            }
            Err(e) => {
                let e = e.to_string();
                if e != last_error {
                    log_warn!(
                        "sem conexão com {addr}: {e}; tentando a cada {}s",
                        RETRY_INTERVAL.as_secs()
                    );
                    last_error = e.clone();
                }
                set_status(
                    generation,
                    Status::Connecting {
                        addr,
                        error: Some(e),
                    },
                );
            }
        }
        wait_retry(generation);
    }
}

type Connected = (
    TcpStream,
    MsgReader<SecureReader<TcpStream>>,
    SecureWriter<TcpStream>,
    (u16, u16),
);

fn open(addr: &str, mode: u8) -> Result<TcpStream, Box<dyn Error>> {
    let target = addr
        .to_socket_addrs()?
        .next()
        .ok_or("endereço sem resolução")?;
    let stream = TcpStream::connect_timeout(&target, CONNECT_TIMEOUT)?;
    stream.set_nodelay(true)?;
    let mut io = Deadline::new(&stream, PEER_TIMEOUT);
    io.write_all(&[mode])?;
    let mut reply = [0u8; 1];
    io.read_exact(&mut reply)?;
    if reply[0] != REPLY_OK {
        return Err(reply_text(reply[0]).into());
    }
    Ok(stream)
}

pub fn pair_request(addr: &str) -> Result<TcpStream, Box<dyn Error>> {
    log_info!("pedindo pareamento a {addr}");
    let stream = open(addr, MODE_PAIR).inspect_err(|e| log_warn!("pareamento recusado: {e}"))?;
    stream.set_read_timeout(Some(PAIR_TIMEOUT))?;
    Ok(stream)
}

pub fn pair_complete(mut stream: TcpStream, code: &str) -> Result<(), Box<dyn Error>> {
    let token = pair_initiator(&mut stream, code.trim())
        .inspect_err(|e| log_warn!("pareamento falhou: {e}"))?;
    keystore::save(&token)?;
    log_info!("pareado com sucesso");
    WAKE.notify_all();
    Ok(())
}

pub fn forget_pairing() -> io::Result<()> {
    keystore::forget()?;
    log_info!("pareamento esquecido");
    drop_link();
    WAKE.notify_all();
    Ok(())
}

fn connect(addr: &str, token: &Key) -> Result<Connected, Box<dyn Error>> {
    let stream = open(addr, MODE_SESSION)?;
    let transport = handshake_initiator(
        &mut Deadline::new(&stream, PEER_TIMEOUT),
        token,
        MODE_SESSION,
    )?;
    stream.set_read_timeout(Some(PEER_TIMEOUT))?;
    let mut reader = MsgReader::new(SecureReader::new(
        stream.try_clone()?,
        Arc::clone(&transport),
    ));
    let writer = SecureWriter::new(stream.try_clone()?, transport);
    match reader.next()? {
        Some(Message::Hello {
            version: PROTOCOL_VERSION,
            width,
            height,
        }) => Ok((stream, reader, writer, (width, height))),
        Some(Message::Hello { version, .. }) => Err(format!(
            "versão de protocolo incompatível: receptor {version}, local {PROTOCOL_VERSION}"
        )
        .into()),
        Some(_) => Err("handshake inesperado: mensagem fora de ordem".into()),
        None => Err("handshake inesperado: conexão encerrada".into()),
    }
}

fn session(
    stream: TcpStream,
    mut reader: MsgReader<SecureReader<TcpStream>>,
    mut writer: SecureWriter<TcpStream>,
    mut link: Link,
    generation: u64,
) -> String {
    let (tx, rx) = mpsc::channel::<Message>();
    let layout = link.switcher.edges().then(|| Message::Layout {
        remote_on_right: link.side == Side::Right,
    });
    let audio_tx = tx.clone();
    link.tx = tx;
    if let Some(ctx) = CTX.lock().unwrap().as_mut() {
        ctx.link = Some(link);
    }
    if CONTROL.lock().unwrap().generation != generation {
        drop_link();
    }

    let clip = Arc::new(Mutex::new(ClipSync::default()));
    let reader_clip = Arc::clone(&clip);
    let left_at: Arc<Mutex<Option<Instant>>> = Arc::default();
    let reader_left_at = Arc::clone(&left_at);
    let reader_thread = thread::spawn(move || {
        let apply = |text: &str| {
            reader_clip.lock().unwrap().incoming(text);
            write_clipboard(text);
        };
        let mut pending: Option<String> = None;
        let reason = loop {
            match reader.next() {
                Ok(Some(Message::MonitorReport { ok, detail })) => {
                    monitor::set_notebook_report(ok, detail);
                }
                Ok(Some(Message::Speaker(pcm))) => audio_win::play(&pcm),
                Ok(Some(Message::AudioReport { ok, detail })) => {
                    audio_win::set_notebook_report(ok, detail);
                }
                Ok(Some(Message::Screens(screens))) => set_remote_screens(screens),
                Ok(Some(Message::Clipboard(text))) if text.len() > MAX_TEXT => {
                    log_warn!("clipboard recebido acima de {MAX_TEXT} bytes; ignorado");
                }
                Ok(Some(Message::Clipboard(text))) => {
                    let replying = reader_left_at
                        .lock()
                        .unwrap()
                        .take()
                        .is_some_and(|t| t.elapsed() < CLIPBOARD_WINDOW);
                    if replying {
                        apply(&text);
                    } else {
                        pending = Some(text);
                    }
                }
                Ok(Some(msg)) => {
                    if on_remote_input(msg, &reader_clip) {
                        if let Some(text) = pending.take() {
                            apply(&text);
                        }
                    }
                }
                Ok(None) => break "conexão encerrada pelo receptor".to_string(),
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    break format!("receptor sem resposta há {}s", PEER_TIMEOUT.as_secs())
                }
                Err(e) => break format!("erro de leitura: {e}"),
            }
        };
        drop_link();
        reason
    });

    let audio = AUDIO.load(Ordering::SeqCst);
    let mut write_error = layout
        .map_or(Ok(()), |layout| write_msg(&mut writer, &layout))
        .and_then(|_| write_msg(&mut writer, &monitor::setup_message()))
        .and_then(|_| write_msg(&mut writer, &monitor::tap_message()))
        .and_then(|_| write_msg(&mut writer, &Message::AudioSetup { enabled: audio }))
        .err();
    if audio && write_error.is_none() {
        audio_win::start(audio_tx);
    } else {
        drop(audio_tx);
    }
    let mut next_ping = Instant::now();
    while write_error.is_none() {
        if Instant::now() >= next_ping {
            if let Err(e) = write_msg(&mut writer, &Message::Ping) {
                write_error = Some(e);
                break;
            }
            next_ping = Instant::now() + PING_INTERVAL;
        }
        match rx.recv_timeout(next_ping.saturating_duration_since(Instant::now())) {
            Ok(msg) => {
                if matches!(msg, Message::Enter { .. } | Message::GiveBack { .. }) {
                    monitor::show_notebook();
                }
                if matches!(msg, Message::Leave) {
                    monitor::cancel();
                    *left_at.lock().unwrap() = Some(Instant::now());
                }
                let text = matches!(msg, Message::Enter { .. })
                    .then(read_clipboard)
                    .flatten()
                    .and_then(|t| clip.lock().unwrap().outgoing(Some(&t)));
                let result = match text {
                    Some(t) => write_msg(&mut writer, &Message::Clipboard(t)),
                    None => Ok(()),
                };
                if let Err(e) = result.and_then(|_| write_msg(&mut writer, &msg)) {
                    write_error = Some(e);
                    break;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }

    drop_link();
    let _ = stream.shutdown(Shutdown::Both);
    let reason = reader_thread
        .join()
        .unwrap_or_else(|_| "falha na leitura".to_string());
    match write_error {
        Some(e) => format!("erro de escrita: {e}"),
        None => reason,
    }
}

fn read_clipboard() -> Option<String> {
    arboard::Clipboard::new().ok()?.get_text().ok()
}

fn write_clipboard(text: &str) {
    let result = arboard::Clipboard::new().and_then(|mut c| c.set_text(to_windows(text)));
    if let Err(e) = result {
        log_warn!("não foi possível gravar o clipboard: {e}");
    }
}

fn drop_link() {
    monitor::cancel();
    audio_win::stop();
    let Some(mut link) = CTX.lock().unwrap().as_mut().and_then(|ctx| ctx.link.take()) else {
        return;
    };
    if let Some(controlled) = link.controlled.take() {
        inject::send(&controlled.release());
    }
    if let Some(Action::Leave { x, y }) = link.switcher.force_leave() {
        let back = link.monitors.iter().find(|m| m.contains(x, y));
        let (x, y) = back.map_or((x, y), |m| arrival(link.pc_spot.get(), *m));
        return_pointer(x, y);
    }
    pointer::show();
}

fn on_remote_input(msg: Message, clip: &Mutex<ClipSync>) -> bool {
    if matches!(msg, Message::TakeOver { .. }) {
        monitor::cancel();
    }
    let mut out = Vec::new();
    let mut took_over = false;
    let mut give_back: Option<(u16, bool)> = None;
    let tx = {
        let mut guard = CTX.lock().unwrap();
        let Some(link) = guard.as_mut().and_then(|ctx| ctx.link.as_mut()) else {
            return false;
        };
        match (msg, link.controlled.as_mut()) {
            (Message::TakeOver { y }, None) if !link.switcher.is_remote() => {
                if let Some(puppet) =
                    Puppet::enter(link.monitors.clone(), link.side, link.preferred, y)
                {
                    let (x, y) = puppet.position();
                    log_info!("notebook assumiu o controle do PC");
                    out.push(Injection::MoveTo(x, y));
                    link.controlled = Some(Controlled {
                        puppet,
                        keys: Vec::new(),
                        buttons: Vec::new(),
                    });
                    took_over = true;
                } else {
                    give_back = Some((y, false));
                }
            }
            (Message::TakeOver { y }, None) => give_back = Some((y, false)),
            (Message::TakeOver { .. }, Some(_)) => {
                log_warn!("TakeOver repetido do notebook ignorado");
            }
            (Message::Delta { dx, dy }, Some(c)) => {
                let (dx, dy) = (
                    (dx as i32).clamp(-MAX_DELTA, MAX_DELTA),
                    (dy as i32).clamp(-MAX_DELTA, MAX_DELTA),
                );
                match c.puppet.motion(dx, dy) {
                    PuppetMove::Move { x, y } => out.push(Injection::MoveTo(x, y)),
                    PuppetMove::Leave { y } => give_back = Some((y, true)),
                    PuppetMove::Stay => {}
                }
            }
            (Message::Button { button, pressed }, Some(c)) => {
                c.buttons.retain(|&b| b != button);
                if pressed {
                    c.buttons.push(button);
                }
                out.push(Injection::Button(button, pressed));
            }
            (Message::Wheel { delta, horizontal }, Some(_)) => {
                out.push(Injection::Wheel(delta, horizontal));
            }
            (Message::Key { code, pressed }, Some(c)) => {
                c.keys.retain(|&k| k != code);
                if pressed {
                    c.keys.push(code);
                }
                out.push(Injection::Key(code, pressed));
            }
            (Message::Abort, Some(_)) => {
                log_info!("notebook cancelou o controle do PC");
                out.extend(
                    link.controlled
                        .take()
                        .map(Controlled::release)
                        .unwrap_or_default(),
                );
            }
            _ => {}
        }
        if give_back.is_some() {
            out.extend(
                link.controlled
                    .take()
                    .map(Controlled::release)
                    .unwrap_or_default(),
            );
        }
        link.tx.clone()
    };
    inject::send(&out);
    if let Some((y, with_clipboard)) = give_back {
        log_info!("controle devolvido ao notebook");
        if with_clipboard {
            let text = read_clipboard().and_then(|t| clip.lock().unwrap().outgoing(Some(&t)));
            if let Some(text) = text {
                let _ = tx.send(Message::Clipboard(text));
            }
        }
        let _ = tx.send(Message::GiveBack { y });
    }
    took_over
}

fn escape_while_controlled(info: &KBDLLHOOKSTRUCT) -> bool {
    let down = |vk: u16| unsafe { GetAsyncKeyState(vk as i32) } as u16 & 0x8000 != 0;
    let pressed = info.flags & LLKHF_UP == 0;
    if !(pressed && info.vkCode == VK_BACK as u32 && down(VK_CONTROL) && down(VK_MENU)) {
        return false;
    }
    let released = {
        let mut guard = CTX.lock().unwrap();
        let Some(link) = guard.as_mut().and_then(|ctx| ctx.link.as_mut()) else {
            return false;
        };
        let Some(controlled) = link.controlled.take() else {
            return false;
        };
        let _ = link.tx.send(Message::GiveBack {
            y: GIVE_BACK_MIDDLE,
        });
        controlled.release()
    };
    log_info!("controle devolvido ao notebook pelo atalho no PC");
    inject::send(&released);
    true
}

type HookProc = unsafe extern "system" fn(i32, WPARAM, LPARAM) -> LRESULT;

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && handle_key(&*(lparam as *const KBDLLHOOKSTRUCT)) {
        return 1;
    }
    CallNextHookEx(ptr::null_mut(), code, wparam, lparam)
}

fn handle_key(info: &KBDLLHOOKSTRUCT) -> bool {
    if info.dwExtraInfo == inject::MAGIC {
        return false;
    }
    let hide = false;
    let mut warp = None;
    let block = {
        let mut guard = CTX.lock().unwrap();
        let Some(ctx) = guard.as_mut() else {
            return false;
        };
        if ctx.link.as_ref().is_some_and(|l| l.controlled.is_some()) {
            drop(guard);
            return escape_while_controlled(info);
        }
        let remote = ctx.link.as_ref().is_some_and(|l| l.switcher.is_remote());
        let pressed = info.flags & LLKHF_UP == 0;
        let code = if info.vkCode == VK_PAUSE as u32 {
            Some(KEY_PAUSE)
        } else {
            evdev_from_windows_scancode(info.scanCode, info.flags & LLKHF_EXTENDED != 0)
        };
        match (code, ctx.link.as_mut()) {
            (None, _) => remote,
            (Some(code), Some(link)) if remote && ctx.keys.is_escape(code, pressed) => {
                if let Some(Action::Leave { x, y }) = link.switcher.force_leave() {
                    let _ = link.tx.send(Message::Leave);
                    warp = Some((x, y));
                }
                true
            }
            (Some(code), link) => match ctx.keys.route(code, pressed, remote) {
                Route::Local => false,
                Route::Drop => true,
                Route::Remote => {
                    if let Some(link) = link {
                        let _ = link.tx.send(Message::Key { code, pressed });
                    }
                    true
                }
            },
        }
    };
    if let Some((x, y)) = warp {
        return_pointer(x, y);
    }
    if hide {
        pointer::hide();
    }
    block
}

type Layout = (Vec<Rect>, (i32, i32));

fn monitors() -> Result<Layout, Box<dyn Error>> {
    unsafe extern "system" fn collect(m: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> i32 {
        let out = &mut *(data as *mut Vec<(Rect, bool)>);
        let mut info: MONITORINFO = mem::zeroed();
        info.cbSize = mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(m, &mut info) != 0 {
            let r = info.rcMonitor;
            out.push((
                Rect {
                    x: r.left,
                    y: r.top,
                    w: r.right - r.left,
                    h: r.bottom - r.top,
                },
                info.dwFlags & MONITORINFOF_PRIMARY != 0,
            ));
        }
        1
    }

    let mut found: Vec<(Rect, bool)> = Vec::new();
    unsafe {
        EnumDisplayMonitors(
            ptr::null_mut(),
            ptr::null(),
            Some(collect),
            &mut found as *mut _ as LPARAM,
        )
    };
    let primary = found
        .iter()
        .find(|(_, p)| *p)
        .or(found.first())
        .map(|(r, _)| *r)
        .ok_or("nenhum monitor encontrado")?;
    let park = (primary.x + primary.w / 2, primary.y + primary.h / 2);
    Ok((found.into_iter().map(|(r, _)| r).collect(), park))
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let info = &*(lparam as *const MSLLHOOKSTRUCT);
        if handle(wparam as u32, info) {
            return 1;
        }
    }
    CallNextHookEx(ptr::null_mut(), code, wparam, lparam)
}

fn handle(msg: u32, info: &MSLLHOOKSTRUCT) -> bool {
    if info.dwExtraInfo == inject::MAGIC {
        return false;
    }
    let mut hide = false;
    let mut warp = None;
    let block = {
        let mut guard = CTX.lock().unwrap();
        let Some(link) = guard.as_mut().and_then(|ctx| ctx.link.as_mut()) else {
            return false;
        };
        if link.controlled.is_some() {
            return false;
        }
        let send = |m: Message| {
            let _ = link.tx.send(m);
        };
        let high = (info.mouseData >> 16) as u16;
        if msg == WM_MOUSEMOVE && link.switcher.is_remote() {
            link.reports.report();
        }
        match msg {
            WM_MOUSEMOVE => match link.switcher.on_motion(info.pt.x, info.pt.y) {
                Action::Pass => {
                    let distance = top_distance(&link.monitors, info.pt.x, info.pt.y);
                    let now = Instant::now();
                    let rest = restful(distance, &link.taps, now);
                    link.pc_spot.observe((info.pt.x, info.pt.y), rest, now);
                    link.watch_top(distance);
                    false
                }
                Action::Block => true,
                Action::Enter { x, y } => {
                    log_info!("controle passou para o notebook");
                    link.ballistics = mouse_speed::ballistics();
                    link.reports.reset();
                    link.taps.reset();
                    send(Message::Enter { x, y });
                    hide = true;
                    true
                }
                Action::Motion { x, y } => {
                    send(Message::Motion { x, y });
                    true
                }
                Action::Leave { x, y } => {
                    send(Message::Leave);
                    warp = Some((x, y));
                    true
                }
            },
            _ if !link.switcher.is_remote() => false,
            WM_LBUTTONDOWN | WM_LBUTTONUP => {
                send(button(Button::Left, msg == WM_LBUTTONDOWN));
                true
            }
            WM_RBUTTONDOWN | WM_RBUTTONUP => {
                send(button(Button::Right, msg == WM_RBUTTONDOWN));
                true
            }
            WM_MBUTTONDOWN | WM_MBUTTONUP => {
                send(button(Button::Middle, msg == WM_MBUTTONDOWN));
                true
            }
            WM_XBUTTONDOWN | WM_XBUTTONUP => {
                let b = if high == XBUTTON1 {
                    Button::Back
                } else {
                    Button::Forward
                };
                send(button(b, msg == WM_XBUTTONDOWN));
                true
            }
            WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
                send(Message::Wheel {
                    delta: high as i16,
                    horizontal: msg == WM_MOUSEHWHEEL,
                });
                true
            }
            _ => true,
        }
    };
    if let Some((x, y)) = warp {
        return_pointer(x, y);
    }
    if hide {
        pointer::hide();
    }
    block
}

fn button(button: Button, pressed: bool) -> Message {
    Message::Button { button, pressed }
}
