use std::collections::HashMap;
use std::error::Error;
use std::io::{self, ErrorKind, Read, Write};
use std::net::{IpAddr, TcpListener, TcpStream};
use std::process::Command;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use lan_kvm_core::clipboard::{ClipSync, MAX_TEXT};
use lan_kvm_core::ddc::edid_name;
use lan_kvm_core::pairing::{
    new_code, pair_responder, reply_text, MODE_PAIR, MODE_SESSION, REPLY_ALREADY_PAIRED,
    REPLY_BUSY, REPLY_LOCKED, REPLY_NOT_PAIRED, REPLY_OK,
};
use lan_kvm_core::proto::{Button, Message, PROTOCOL_VERSION};
use lan_kvm_core::secure::{handshake_responder, SecureReader, SecureWriter};
use lan_kvm_core::switch::Rect;
use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xproto::{
    AtomEnum, ConnectionExt as _, BUTTON_PRESS_EVENT, KEY_PRESS_EVENT, MOTION_NOTIFY_EVENT,
};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;
use x11rb::CURRENT_TIME;

use crate::audio_linux::{AudioBridge, Outbox};
use crate::capture::Capture;
use crate::ddc_linux::{switch_now, MonitorSwitch};
use crate::keystore;
use crate::net::{write_msg, Deadline, MsgReader, PEER_TIMEOUT};

const WHEEL_STEP: i32 = 120;
const GREETING_TIMEOUT: Duration = Duration::from_secs(10);
const CODE_TIMEOUT: Duration = Duration::from_secs(150);
const PAIRING_COOLDOWN: Duration = Duration::from_secs(10);
const MAX_PAIRING_FAILURES: u32 = 5;
const MAX_PENDING_PER_IP: usize = 4;
const MAX_PENDING: usize = 32;
const EVDEV_OFFSET: u16 = 8;

struct Injector {
    conn: RustConnection,
    root: u32,
    held: Vec<(u8, u8)>,
    wheel: [i32; 2],
}

impl Injector {
    fn screens(&self) -> Vec<(Rect, String)> {
        let edid = self
            .conn
            .intern_atom(true, b"EDID")
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|r| r.atom);
        let Some(monitors) = self
            .conn
            .randr_get_monitors(self.root, true)
            .ok()
            .and_then(|c| c.reply().ok())
        else {
            return Vec::new();
        };
        monitors
            .monitors
            .iter()
            .map(|m| {
                let rect = Rect {
                    x: m.x.into(),
                    y: m.y.into(),
                    w: m.width.into(),
                    h: m.height.into(),
                };
                let from_edid = edid.zip(m.outputs.first()).and_then(|(atom, &output)| {
                    let reply = self
                        .conn
                        .randr_get_output_property(output, atom, AtomEnum::ANY, 0, 64, false, false)
                        .ok()?
                        .reply()
                        .ok()?;
                    edid_name(&reply.data)
                });
                let name = from_edid.unwrap_or_else(|| {
                    self.conn
                        .get_atom_name(m.name)
                        .ok()
                        .and_then(|c| c.reply().ok())
                        .map(|r| String::from_utf8_lossy(&r.name).into_owned())
                        .unwrap_or_default()
                });
                (rect, name)
            })
            .collect()
    }

    fn motion(&self, x: u16, y: u16) -> Result<(), Box<dyn Error>> {
        self.conn.xtest_fake_input(
            MOTION_NOTIFY_EVENT,
            0,
            CURRENT_TIME,
            self.root,
            x as i16,
            y as i16,
            0,
        )?;
        Ok(())
    }

    fn press(&mut self, press_kind: u8, code: u8, pressed: bool) -> Result<(), Box<dyn Error>> {
        let kind = if pressed { press_kind } else { press_kind + 1 };
        self.conn
            .xtest_fake_input(kind, code, CURRENT_TIME, x11rb::NONE, 0, 0, 0)?;
        self.held.retain(|&held| held != (press_kind, code));
        if pressed {
            self.held.push((press_kind, code));
        }
        Ok(())
    }

    fn button(&mut self, code: u8, pressed: bool) -> Result<(), Box<dyn Error>> {
        self.press(BUTTON_PRESS_EVENT, code, pressed)
    }

    fn key(&mut self, evdev: u16, pressed: bool) -> Result<(), Box<dyn Error>> {
        match evdev
            .checked_add(EVDEV_OFFSET)
            .and_then(|c| u8::try_from(c).ok())
        {
            Some(code) => self.press(KEY_PRESS_EVENT, code, pressed),
            None => Ok(()),
        }
    }

    fn wheel(&mut self, delta: i16, horizontal: bool) -> Result<(), Box<dyn Error>> {
        let (axis, positive, negative) = if horizontal { (1, 7, 6) } else { (0, 4, 5) };
        self.wheel[axis] += delta as i32;
        while self.wheel[axis].abs() >= WHEEL_STEP {
            let code = if self.wheel[axis] > 0 {
                positive
            } else {
                negative
            };
            self.wheel[axis] -= WHEEL_STEP * self.wheel[axis].signum();
            self.button(code, true)?;
            self.button(code, false)?;
        }
        Ok(())
    }

    fn release_all(&mut self) -> Result<(), Box<dyn Error>> {
        for (press_kind, code) in std::mem::take(&mut self.held) {
            self.press(press_kind, code, false)?;
        }
        self.held.clear();
        self.wheel = [0; 2];
        self.conn.flush()?;
        Ok(())
    }

    fn apply(&mut self, msg: Message) -> Result<(), Box<dyn Error>> {
        match msg {
            Message::Enter { x, y } | Message::Motion { x, y } => self.motion(x, y)?,
            Message::Button { button, pressed } => self.button(x11_button(button), pressed)?,
            Message::Wheel { delta, horizontal } => self.wheel(delta, horizontal)?,
            Message::Key { code, pressed } => self.key(code, pressed)?,
            Message::Leave => self.release_all()?,
            Message::Hello { .. }
            | Message::Ping
            | Message::Pong
            | Message::Clipboard(_)
            | Message::Layout { .. }
            | Message::TakeOver { .. }
            | Message::Delta { .. }
            | Message::GiveBack { .. }
            | Message::Abort
            | Message::MonitorSetup { .. }
            | Message::MonitorReport { .. }
            | Message::MonitorSwitch { .. }
            | Message::AudioSetup { .. }
            | Message::Speaker(_)
            | Message::Mic(_)
            | Message::AudioReport { .. }
            | Message::Screens(_)
            | Message::TapSetup { .. } => {}
        }
        self.conn.flush()?;
        while self.conn.poll_for_event()?.is_some() {}
        Ok(())
    }
}

pub struct SharedClipboard {
    inner: Option<arboard::Clipboard>,
    sync: ClipSync,
}

impl SharedClipboard {
    fn new() -> Self {
        let inner = arboard::Clipboard::new()
            .map_err(|e| log_warn!("clipboard indisponível: {e}"))
            .ok();
        Self {
            inner,
            sync: ClipSync::default(),
        }
    }

    fn set(&mut self, text: &str) {
        self.sync.incoming(text);
        if let Some(Err(e)) = self.inner.as_mut().map(|c| c.set_text(text)) {
            log_warn!("não foi possível gravar o clipboard: {e}");
        }
    }

    pub fn changed(&mut self) -> Option<String> {
        let text = self.inner.as_mut()?.get_text().ok();
        self.sync.outgoing(text.as_deref())
    }
}

fn x11_button(b: Button) -> u8 {
    match b {
        Button::Left => 1,
        Button::Middle => 2,
        Button::Right => 3,
        Button::Back => 8,
        Button::Forward => 9,
    }
}

fn block_stop_signals() -> libc::sigset_t {
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        for sig in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
            libc::sigaddset(&mut set, sig);
        }
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
        set
    }
}

fn watch_stop_signals(set: libc::sigset_t, monitor: Arc<MonitorSwitch>, audio: Arc<AudioBridge>) {
    thread::spawn(move || {
        let mut sig = 0;
        unsafe { libc::sigwait(&set, &mut sig) };
        log_info!("receptor encerrado pelo sistema (sinal {sig})");
        monitor.go_home();
        audio.disable();
        std::process::exit(0);
    });
}

pub fn run_receiver(bind: &str) -> Result<(), Box<dyn Error>> {
    let signals = block_stop_signals();
    let monitor = MonitorSwitch::new();
    let audio = AudioBridge::new();
    watch_stop_signals(signals, Arc::clone(&monitor), Arc::clone(&audio));
    let result = receive(bind, &monitor, &audio);
    if result.is_err() {
        monitor.go_home();
        audio.disable();
    }
    result
}

fn receive(
    bind: &str,
    monitor: &Arc<MonitorSwitch>,
    audio: &Arc<AudioBridge>,
) -> Result<(), Box<dyn Error>> {
    let (conn, screen_num) = RustConnection::connect(None)?;
    conn.xtest_get_version(2, 2)?.reply()?;
    let screen = &conn.setup().roots[screen_num];
    let (root, width, height) = (screen.root, screen.width_in_pixels, screen.height_in_pixels);
    let mut injector = Injector {
        conn,
        root,
        held: Vec::new(),
        wheel: [0; 2],
    };
    let clipboard = Arc::new(Mutex::new(SharedClipboard::new()));

    let listener = TcpListener::bind(bind)?;
    log_info!("aguardando em {bind}, tela {width}x{height}");
    let (sessions, queue) = mpsc::sync_channel::<(Session, String)>(2);
    let gate = Arc::new(Mutex::new(Gate::default()));
    thread::spawn(move || accept_loop(listener, gate, sessions));

    for (session, peer) in queue {
        log_info!("sessão iniciada com {peer}");
        let result = serve(
            &mut injector,
            &clipboard,
            monitor,
            audio,
            session,
            width,
            height,
        );
        monitor.session_ended();
        audio.disable();
        if let Err(e) = result {
            log_warn!("erro na sessão com {peer}: {e}");
        }
        injector.release_all()?;
        log_info!("sessão encerrada com {peer}");
    }
    Ok(())
}

#[derive(Default)]
struct Gate {
    pending: HashMap<IpAddr, usize>,
    total: usize,
    pairing_busy: bool,
    last_pairing: Option<Instant>,
    pairing_failures: u32,
}

impl Gate {
    fn admit(&mut self, ip: IpAddr) -> bool {
        let count = self.pending.entry(ip).or_default();
        if *count >= MAX_PENDING_PER_IP || self.total >= MAX_PENDING {
            return false;
        }
        *count += 1;
        self.total += 1;
        true
    }

    fn release(&mut self, ip: IpAddr) {
        if let Some(count) = self.pending.get_mut(&ip) {
            *count -= 1;
            if *count == 0 {
                self.pending.remove(&ip);
            }
        }
        self.total = self.total.saturating_sub(1);
    }

    fn begin_pairing(&mut self, now: Instant) -> Result<(), u8> {
        if self.pairing_failures >= MAX_PAIRING_FAILURES {
            return Err(REPLY_LOCKED);
        }
        if self.pairing_busy
            || self
                .last_pairing
                .is_some_and(|t| now - t < PAIRING_COOLDOWN)
        {
            return Err(REPLY_BUSY);
        }
        self.pairing_busy = true;
        self.last_pairing = Some(now);
        Ok(())
    }

    fn end_pairing(&mut self, wrong_code: bool) -> bool {
        self.pairing_busy = false;
        if wrong_code {
            self.pairing_failures += 1;
        }
        wrong_code && self.pairing_failures == MAX_PAIRING_FAILURES
    }
}

fn accept_loop(
    listener: TcpListener,
    gate: Arc<Mutex<Gate>>,
    sessions: mpsc::SyncSender<(Session, String)>,
) {
    for stream in listener.incoming() {
        let stream = match stream {
            Ok(s) => s,
            Err(e) => {
                log_error!("erro ao aceitar conexão: {e}");
                continue;
            }
        };
        let Ok(peer) = stream.peer_addr() else {
            continue;
        };
        if !gate.lock().unwrap().admit(peer.ip()) {
            log_warn!("conexão de {peer} recusada: conexões pendentes demais");
            continue;
        }
        let gate = Arc::clone(&gate);
        let sessions = sessions.clone();
        thread::spawn(move || {
            let result = greet(stream, &gate);
            gate.lock().unwrap().release(peer.ip());
            match result {
                Ok(Some(session)) => {
                    if sessions.try_send((session, peer.to_string())).is_err() {
                        log_warn!("sessão de {peer} descartada: receptor ocupado");
                    }
                }
                Ok(None) => {}
                Err(e) => log_warn!("conexão recusada ({peer}): {e}"),
            }
        });
    }
}

pub fn unpair() -> Result<(), Box<dyn Error>> {
    keystore::forget()?;
    log_info!("pareamento removido; o próximo PC que pedir poderá parear");
    Ok(())
}

fn notify(summary: &str, body: &str) {
    let _ = Command::new("notify-send")
        .args(["--app-name=lan-kvm", "--expire-time=120000", summary, body])
        .spawn()
        .and_then(|mut child| child.wait());
}

type Session = (SecureReader<TcpStream>, SecureWriter<TcpStream>);

fn greet(stream: TcpStream, gate: &Mutex<Gate>) -> Result<Option<Session>, Box<dyn Error>> {
    stream.set_nodelay(true)?;
    let mut io = Deadline::new(&stream, GREETING_TIMEOUT);
    let mut mode = [0u8; 1];
    io.read_exact(&mut mode)?;
    match mode[0] {
        MODE_SESSION => {
            let token = match keystore::load() {
                Ok(Some(token)) => token,
                Ok(None) => {
                    io.write_all(&[REPLY_NOT_PAIRED])?;
                    return Err("notebook ainda não pareado".into());
                }
                Err(e) => {
                    io.write_all(&[REPLY_NOT_PAIRED])?;
                    return Err(format!("não foi possível ler o pareamento: {e}").into());
                }
            };
            io.write_all(&[REPLY_OK])?;
            let transport = handshake_responder(&mut io, &token, MODE_SESSION)?;
            let reader = SecureReader::new(stream.try_clone()?, Arc::clone(&transport));
            Ok(Some((reader, SecureWriter::new(stream, transport))))
        }
        MODE_PAIR => {
            match keystore::load() {
                Ok(None) => {}
                Ok(Some(_)) => {
                    io.write_all(&[REPLY_ALREADY_PAIRED])?;
                    return Err("pedido de pareamento recusado: já pareado".into());
                }
                Err(e) => {
                    io.write_all(&[REPLY_ALREADY_PAIRED])?;
                    return Err(format!(
                        "pedido de pareamento recusado: não foi possível ler o pareamento ({e}); \
                         rode `lan-kvm unpair` para começar de novo"
                    )
                    .into());
                }
            }
            if let Err(reply) = gate.lock().unwrap().begin_pairing(Instant::now()) {
                io.write_all(&[reply])?;
                return Err(format!("pedido de pareamento recusado: {}", reply_text(reply)).into());
            }
            let result = pair(&stream);
            let wrong_code = matches!(&result, Err(e) if e.kind() == ErrorKind::PermissionDenied);
            if gate.lock().unwrap().end_pairing(wrong_code) {
                log_warn!("pareamento bloqueado após {MAX_PAIRING_FAILURES} códigos errados");
                notify(
                    "lan-kvm bloqueou o pareamento",
                    "Códigos errados demais. Reinicie a sessão (ou o receptor) para tentar de novo.",
                );
            }
            result?;
            Ok(None)
        }
        other => Err(format!("modo de conexão desconhecido: {other}").into()),
    }
}

fn pair(stream: &TcpStream) -> io::Result<()> {
    let code = new_code()?;
    log_info!("código de pareamento: {code}");
    notify(
        &format!("Código de pareamento do lan-kvm: {code}"),
        "Digite este código no PC para parear.",
    );
    let mut io = Deadline::new(stream, CODE_TIMEOUT);
    io.write_all(&[REPLY_OK])?;
    let token = pair_responder(&mut io, &code)?;
    keystore::save(&token)?;
    log_info!("pareado com sucesso");
    notify(
        "lan-kvm pareado",
        "Este notebook agora aceita o PC pareado.",
    );
    Ok(())
}

fn serve(
    injector: &mut Injector,
    clipboard: &Arc<Mutex<SharedClipboard>>,
    monitor: &Arc<MonitorSwitch>,
    audio: &AudioBridge,
    (reader, writer): Session,
    width: u16,
    height: u16,
) -> Result<(), Box<dyn Error>> {
    let writer = Arc::new(Mutex::new(writer));
    let send = |msg: &Message| write_msg(&mut *writer.lock().unwrap(), msg);
    send(&Message::Hello {
        version: PROTOCOL_VERSION,
        width,
        height,
    })?;
    send(&Message::Screens(injector.screens()))?;
    writer
        .lock()
        .unwrap()
        .get_ref()
        .set_read_timeout(Some(PEER_TIMEOUT))?;
    let mut reader = MsgReader::new(reader);
    let mut capture: Option<Capture> = None;
    let mut session = SessionState::default();
    loop {
        let msg = match reader.next() {
            Ok(Some(msg)) => msg,
            Ok(None) => return Ok(()),
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                return Err(format!("emissor sem resposta há {}s", PEER_TIMEOUT.as_secs()).into())
            }
            Err(e) => return Err(e.into()),
        };
        if let Message::MonitorSetup {
            enabled,
            pc_input,
            delay_ms,
            name,
            ..
        } = msg
        {
            let delay = Duration::from_millis(delay_ms.into());
            if let Some(report) = monitor.configure(enabled, name, pc_input, delay) {
                send(&report)?;
            }
            continue;
        }
        if let Message::TapSetup { pc_input, name } = msg {
            monitor.set_tap_home(name, pc_input);
            continue;
        }
        if let Message::MonitorSwitch { value, name } = msg {
            send(&switch_now(&name, value))?;
            continue;
        }
        if let Message::Mic(pcm) = msg {
            audio.mic(pcm);
            continue;
        }
        if let Message::AudioSetup { enabled } = msg {
            let shared = Arc::clone(&writer);
            let outbox: Outbox =
                Arc::new(move |m: &Message| write_msg(&mut *shared.lock().unwrap(), m));
            if let Some(report) = audio.set(enabled, outbox) {
                send(&report)?;
            }
            continue;
        }
        let grabbed = capture.as_ref().is_some_and(Capture::is_grabbed);
        match session.route(msg, grabbed) {
            Step::Ignore => {}
            Step::Pong => send(&Message::Pong)?,
            Step::StartCapture { remote_on_right } if capture.is_none() => {
                match Capture::start(
                    Arc::clone(&writer),
                    Arc::clone(clipboard),
                    Arc::clone(monitor),
                    remote_on_right,
                ) {
                    Ok(c) => capture = Some(c),
                    Err(e) => log_error!("controle a partir do notebook indisponível: {e}"),
                }
            }
            Step::StartCapture { .. } => {}
            Step::Enter { clip, msg } => {
                monitor.notebook_shown();
                if let Some(text) = clip {
                    clipboard.lock().unwrap().set(&text);
                }
                if let Some(c) = &capture {
                    c.disarm();
                }
                injector.apply(msg)?;
            }
            Step::Inject(msg) => injector.apply(msg)?,
            Step::Leave => {
                monitor.show_pc();
                injector.apply(Message::Leave)?;
                let text = clipboard.lock().unwrap().changed();
                if let Some(text) = text {
                    send(&Message::Clipboard(text))?;
                }
                if let Some(c) = &capture {
                    c.arm();
                }
            }
            Step::GiveBack { y, clip } => {
                monitor.notebook_shown();
                if let Some(text) = clip {
                    clipboard.lock().unwrap().set(&text);
                }
                if let Some(c) = &capture {
                    c.give_back(y);
                }
            }
            Step::Oversized => log_warn!("clipboard recebido acima de {MAX_TEXT} bytes; ignorado"),
        }
    }
}

#[derive(Default)]
struct SessionState {
    active: bool,
    pending_clip: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
enum Step {
    Ignore,
    Pong,
    StartCapture { remote_on_right: bool },
    Enter { clip: Option<String>, msg: Message },
    Inject(Message),
    Leave,
    GiveBack { y: u16, clip: Option<String> },
    Oversized,
}

impl SessionState {
    fn route(&mut self, msg: Message, grabbed: bool) -> Step {
        match msg {
            Message::Ping => Step::Pong,
            Message::Layout { remote_on_right } => Step::StartCapture { remote_on_right },
            Message::Clipboard(text) if text.len() > MAX_TEXT => Step::Oversized,
            Message::Clipboard(text) => {
                self.pending_clip = Some(text);
                Step::Ignore
            }
            msg @ Message::Enter { .. } => {
                self.active = true;
                Step::Enter {
                    clip: self.pending_clip.take(),
                    msg,
                }
            }
            msg @ (Message::Motion { .. }
            | Message::Button { .. }
            | Message::Wheel { .. }
            | Message::Key { .. })
                if self.active =>
            {
                Step::Inject(msg)
            }
            Message::Leave if self.active => {
                self.active = false;
                self.pending_clip = None;
                Step::Leave
            }
            Message::GiveBack { y } if grabbed => Step::GiveBack {
                y,
                clip: self.pending_clip.take(),
            },
            _ => Step::Ignore,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lan_kvm_core::proto::Button;

    fn key() -> Message {
        Message::Key {
            code: 30,
            pressed: true,
        }
    }

    #[test]
    fn input_is_ignored_unless_the_pc_has_entered() {
        let mut s = SessionState::default();
        assert_eq!(s.route(Message::Motion { x: 0, y: 5 }, false), Step::Ignore);
        assert_eq!(s.route(key(), false), Step::Ignore);
        assert_eq!(s.route(Message::Leave, false), Step::Ignore);
        let enter = Message::Enter { x: 0, y: 1 };
        assert_eq!(
            s.route(enter.clone(), false),
            Step::Enter {
                clip: None,
                msg: enter
            }
        );
        assert_eq!(s.route(key(), false), Step::Inject(key()));
        let click = Message::Button {
            button: Button::Left,
            pressed: true,
        };
        assert_eq!(s.route(click.clone(), false), Step::Inject(click));
        assert_eq!(s.route(Message::Leave, false), Step::Leave);
        assert_eq!(s.route(key(), false), Step::Ignore);
        assert_eq!(s.route(Message::Leave, false), Step::Ignore);
    }

    #[test]
    fn clipboard_is_applied_only_on_a_real_transition() {
        let mut s = SessionState::default();
        assert_eq!(s.route(Message::Clipboard("x".into()), false), Step::Ignore);
        assert_eq!(s.route(Message::GiveBack { y: 1 }, false), Step::Ignore);
        let enter = Message::Enter { x: 0, y: 0 };
        assert_eq!(
            s.route(enter.clone(), false),
            Step::Enter {
                clip: Some("x".into()),
                msg: enter
            }
        );
        assert_eq!(s.route(Message::Leave, false), Step::Leave);
        assert_eq!(s.route(Message::Clipboard("y".into()), false), Step::Ignore);
        assert_eq!(
            s.route(Message::GiveBack { y: 7 }, true),
            Step::GiveBack {
                y: 7,
                clip: Some("y".into())
            }
        );
        assert_eq!(
            s.route(Message::Clipboard("z".repeat(MAX_TEXT + 1)), false),
            Step::Oversized
        );
    }

    #[test]
    fn pairing_gate_limits_guesses_and_concurrency() {
        let mut g = Gate::default();
        let t0 = Instant::now();
        assert_eq!(g.begin_pairing(t0), Ok(()));
        assert_eq!(g.begin_pairing(t0 + PAIRING_COOLDOWN), Err(REPLY_BUSY));
        assert!(!g.end_pairing(false));
        assert_eq!(
            g.begin_pairing(t0 + Duration::from_secs(1)),
            Err(REPLY_BUSY)
        );
        let mut t = t0;
        for attempt in 1..=MAX_PAIRING_FAILURES {
            t += PAIRING_COOLDOWN;
            assert_eq!(g.begin_pairing(t), Ok(()));
            assert_eq!(g.end_pairing(true), attempt == MAX_PAIRING_FAILURES);
        }
        assert_eq!(
            g.begin_pairing(t + PAIRING_COOLDOWN * 10),
            Err(REPLY_LOCKED)
        );
    }

    #[test]
    fn gate_caps_pending_connections_per_ip_and_total() {
        let mut g = Gate::default();
        let a: IpAddr = "10.0.0.1".parse().unwrap();
        for _ in 0..MAX_PENDING_PER_IP {
            assert!(g.admit(a));
        }
        assert!(!g.admit(a));
        g.release(a);
        assert!(g.admit(a));
        for i in 0..(MAX_PENDING - MAX_PENDING_PER_IP) {
            let ip: IpAddr = format!("10.0.1.{}", i % 250).parse().unwrap();
            g.admit(ip);
        }
        assert!(!g.admit("10.0.2.1".parse().unwrap()));
    }
}
