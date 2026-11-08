use std::error::Error;
use std::io;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use lan_kvm_core::proto::{Button, Message};
use lan_kvm_core::puppet::{arrival_x, at_facing_edge, from_permille, to_permille};
use lan_kvm_core::secure::SecureWriter;
use x11rb::connection::Connection;
use x11rb::protocol::xinput::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{
    ConnectionExt as _, CreateGCAux, EventMask, GrabMode, GrabStatus, Rectangle,
};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::{CURRENT_TIME, NONE};

use crate::ddc_linux::MonitorSwitch;
use crate::net::write_msg;
use crate::x11::SharedClipboard;

const IDLE: Duration = Duration::from_millis(2);
const KEY_BACKSPACE: u16 = 14;
const CTRL: [u16; 2] = [29, 97];
const ALT: [u16; 2] = [56, 100];

pub type Writer = Arc<Mutex<SecureWriter<TcpStream>>>;
pub type Clipboard = Arc<Mutex<SharedClipboard>>;

enum Cmd {
    Arm,
    GiveBack(u16),
}

pub struct Capture {
    armed: Arc<AtomicBool>,
    grabbed: Arc<AtomicBool>,
    tx: Option<Sender<Cmd>>,
    thread: Option<JoinHandle<()>>,
}

impl Capture {
    pub fn start(
        writer: Writer,
        clipboard: Clipboard,
        monitor: Arc<MonitorSwitch>,
        remote_on_right: bool,
    ) -> Result<Self, Box<dyn Error>> {
        let (conn, screen_num) = RustConnection::connect(None)?;
        conn.xinput_xi_query_version(2, 2)?.reply()?;
        let screen = &conn.setup().roots[screen_num];
        let (root, width, height) = (screen.root, screen.width_in_pixels, screen.height_in_pixels);
        conn.xinput_xi_select_events(
            root,
            &[xinput::EventMask {
                deviceid: xinput::Device::ALL_MASTER.into(),
                mask: vec![xinput::XIEventMask::RAW_MOTION],
            }],
        )?;
        let pixmap = conn.generate_id()?;
        conn.create_pixmap(1, pixmap, root, 1, 1)?;
        let gc = conn.generate_id()?;
        conn.create_gc(gc, pixmap, &CreateGCAux::new().foreground(0))?;
        conn.poly_fill_rectangle(
            pixmap,
            gc,
            &[Rectangle {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            }],
        )?;
        let cursor = conn.generate_id()?;
        conn.create_cursor(cursor, pixmap, pixmap, 0, 0, 0, 0, 0, 0, 0, 0)?;
        conn.flush()?;

        let armed = Arc::new(AtomicBool::new(true));
        let grabbed = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let mut grabber = Grabber {
            conn,
            root,
            width: width as i32,
            height: height as i32,
            remote_on_right,
            cursor,
            center: ((width / 2) as i16, (height / 2) as i16),
            grabbed: false,
            grabbed_flag: Arc::clone(&grabbed),
            grab_refused: false,
            held: Vec::new(),
            writer,
            clipboard,
            monitor,
        };
        let thread_armed = Arc::clone(&armed);
        let thread = thread::spawn(move || {
            if let Err(e) = grabber.run(&rx, &thread_armed) {
                log_error!("captura encerrada: {e}");
            }
            let _ = grabber.release(None);
        });
        Ok(Self {
            armed,
            grabbed,
            tx: Some(tx),
            thread: Some(thread),
        })
    }

    pub fn is_grabbed(&self) -> bool {
        self.grabbed.load(Ordering::SeqCst)
    }

    pub fn disarm(&self) {
        self.armed.store(false, Ordering::SeqCst);
    }

    pub fn arm(&self) {
        self.armed.store(true, Ordering::SeqCst);
        self.send(Cmd::Arm);
    }

    pub fn give_back(&self, y: u16) {
        self.send(Cmd::GiveBack(y));
    }

    fn send(&self, cmd: Cmd) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(cmd);
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.tx = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Grabber {
    conn: RustConnection,
    root: u32,
    width: i32,
    height: i32,
    remote_on_right: bool,
    cursor: u32,
    center: (i16, i16),
    grabbed: bool,
    grabbed_flag: Arc<AtomicBool>,
    grab_refused: bool,
    held: Vec<u16>,
    writer: Writer,
    clipboard: Clipboard,
    monitor: Arc<MonitorSwitch>,
}

impl Grabber {
    fn run(&mut self, rx: &Receiver<Cmd>, armed: &AtomicBool) -> Result<(), Box<dyn Error>> {
        loop {
            loop {
                match rx.try_recv() {
                    Ok(Cmd::Arm) => self.nudge_off_edge()?,
                    Ok(Cmd::GiveBack(y)) => self.release(Some(y))?,
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return Ok(()),
                }
            }
            let mut busy = false;
            let mut motion = None;
            while let Some(event) = self.conn.poll_for_event()? {
                busy = true;
                match event {
                    Event::XinputRawMotion(_) if !self.grabbed && armed.load(Ordering::SeqCst) => {
                        let pointer = self.conn.query_pointer(self.root)?.reply()?;
                        if armed.load(Ordering::SeqCst)
                            && at_facing_edge(
                                pointer.root_x as i32,
                                self.width,
                                self.remote_on_right,
                            )
                        {
                            self.take_over(pointer.root_y as i32)?;
                        }
                    }
                    Event::MotionNotify(e) if self.grabbed => motion = Some((e.root_x, e.root_y)),
                    Event::ButtonPress(e) if self.grabbed => self.button(e.detail, true)?,
                    Event::ButtonRelease(e) if self.grabbed => self.button(e.detail, false)?,
                    Event::KeyPress(e) if self.grabbed => self.key(e.detail, true)?,
                    Event::KeyRelease(e) if self.grabbed => self.key(e.detail, false)?,
                    _ => {}
                }
            }
            if let Some(position) = motion {
                self.motion(position)?;
            }
            if !busy {
                thread::sleep(IDLE);
            }
        }
    }

    fn send(&self, msg: &Message) -> io::Result<()> {
        write_msg(&mut *self.writer.lock().unwrap(), msg)
    }

    fn warp(&self, x: i32, y: i32) -> Result<(), Box<dyn Error>> {
        self.conn
            .warp_pointer(NONE, self.root, 0, 0, 0, 0, x as i16, y as i16)?;
        self.conn.flush()?;
        Ok(())
    }

    fn take_over(&mut self, y: i32) -> Result<(), Box<dyn Error>> {
        let mask = EventMask::POINTER_MOTION | EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE;
        let pointer = self
            .conn
            .grab_pointer(
                false,
                self.root,
                mask,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
                NONE,
                self.cursor,
                CURRENT_TIME,
            )?
            .reply()?;
        let keyboard = self
            .conn
            .grab_keyboard(
                false,
                self.root,
                CURRENT_TIME,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
            )?
            .reply()?;
        if pointer.status != GrabStatus::SUCCESS || keyboard.status != GrabStatus::SUCCESS {
            self.conn.ungrab_pointer(CURRENT_TIME)?;
            self.conn.ungrab_keyboard(CURRENT_TIME)?;
            if !self.grab_refused {
                log_warn!(
                    "outro programa está com o mouse e o teclado ({:?}, {:?}); \
                     o controle do PC fica indisponível até ele liberar",
                    pointer.status,
                    keyboard.status
                );
                self.grab_refused = true;
            }
            return self.warp(arrival_x(self.width, self.remote_on_right), y);
        }
        self.grab_refused = false;
        self.monitor.show_pc();
        let text = self.clipboard.lock().unwrap().changed();
        if let Some(text) = text {
            self.send(&Message::Clipboard(text))?;
        }
        self.send(&Message::TakeOver {
            y: to_permille(y, self.height),
        })?;
        self.grabbed = true;
        self.grabbed_flag.store(true, Ordering::SeqCst);
        self.held.clear();
        self.warp(self.center.0 as i32, self.center.1 as i32)
    }

    fn motion(&mut self, (x, y): (i16, i16)) -> Result<(), Box<dyn Error>> {
        let (dx, dy) = (x - self.center.0, y - self.center.1);
        if (dx, dy) == (0, 0) {
            return Ok(());
        }
        self.send(&Message::Delta { dx, dy })?;
        self.warp(self.center.0 as i32, self.center.1 as i32)
    }

    fn button(&mut self, detail: u8, pressed: bool) -> Result<(), Box<dyn Error>> {
        let button = match detail {
            1 => Button::Left,
            2 => Button::Middle,
            3 => Button::Right,
            8 => Button::Back,
            9 => Button::Forward,
            4..=7 if pressed => {
                let delta = if matches!(detail, 4 | 7) { 120 } else { -120 };
                self.send(&Message::Wheel {
                    delta,
                    horizontal: detail >= 6,
                })?;
                return Ok(());
            }
            _ => return Ok(()),
        };
        self.send(&Message::Button { button, pressed })?;
        Ok(())
    }

    fn key(&mut self, detail: u8, pressed: bool) -> Result<(), Box<dyn Error>> {
        let Some(code) = (detail as u16).checked_sub(8) else {
            return Ok(());
        };
        let held = |keys: [u16; 2], list: &[u16]| keys.iter().any(|k| list.contains(k));
        if pressed && code == KEY_BACKSPACE && held(CTRL, &self.held) && held(ALT, &self.held) {
            self.send(&Message::Abort)?;
            return self.release(None);
        }
        self.held.retain(|&k| k != code);
        if pressed {
            self.held.push(code);
        }
        self.send(&Message::Key { code, pressed })?;
        Ok(())
    }

    fn release(&mut self, y: Option<u16>) -> Result<(), Box<dyn Error>> {
        if !self.grabbed {
            return Ok(());
        }
        self.grabbed = false;
        self.grabbed_flag.store(false, Ordering::SeqCst);
        self.held.clear();
        self.conn.ungrab_pointer(CURRENT_TIME)?;
        self.conn.ungrab_keyboard(CURRENT_TIME)?;
        let y = y.map_or(self.center.1 as i32, |y| from_permille(y, self.height));
        self.warp(arrival_x(self.width, self.remote_on_right), y)
    }

    fn nudge_off_edge(&mut self) -> Result<(), Box<dyn Error>> {
        let pointer = self.conn.query_pointer(self.root)?.reply()?;
        if at_facing_edge(pointer.root_x as i32, self.width, self.remote_on_right) {
            self.warp(
                arrival_x(self.width, self.remote_on_right),
                pointer.root_y as i32,
            )?;
        }
        Ok(())
    }
}
