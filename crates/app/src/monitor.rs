use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use lan_kvm_core::ddc::{input_name, spawn_switcher, Request, SWITCH_DELAY};
use lan_kvm_core::proto::Message;

use crate::ddc_win;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Choice {
    pub enabled: bool,
    pub name: String,
    pub pc_input: u8,
    pub notebook_input: u8,
    pub guard: bool,
}

impl Choice {
    fn delay(&self) -> Duration {
        if self.guard {
            SWITCH_DELAY
        } else {
            Duration::ZERO
        }
    }

    fn active(&self) -> bool {
        self.enabled && !self.name.is_empty() && self.pc_input != 0 && self.notebook_input != 0
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TapChoice {
    pub enabled: bool,
    pub name: String,
    pub pc_input: u8,
    pub notebook_input: u8,
}

impl TapChoice {
    pub fn active(&self) -> bool {
        self.enabled && !self.name.is_empty() && self.pc_input != 0 && self.notebook_input != 0
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum TapMove {
    Local(u8),
    Notebook(u8),
}

pub fn tap_move(choice: &TapChoice, current_on_pc: Option<u8>) -> TapMove {
    match current_on_pc {
        Some(v) if v != choice.notebook_input => TapMove::Local(choice.notebook_input),
        _ => TapMove::Notebook(choice.pc_input),
    }
}

static CHOICE: Mutex<Option<Choice>> = Mutex::new(None);
static TAP: Mutex<Option<TapChoice>> = Mutex::new(None);
static TAP_BUSY: AtomicBool = AtomicBool::new(false);
static NOTEBOOK: Mutex<Option<(bool, String)>> = Mutex::new(None);
static PC: Mutex<Option<(bool, String)>> = Mutex::new(None);
static WORKER: OnceLock<Mutex<Sender<Request>>> = OnceLock::new();

fn request(request: Request) {
    let worker = WORKER.get_or_init(|| {
        Mutex::new(spawn_switcher(|name, value| {
            let label = input_name(value);
            let result = match ddc_win::set_input(name, value) {
                Ok(()) => {
                    log_info!("monitor {name} trocado para {label}");
                    (true, format!("trocou para {label}"))
                }
                Err(e) => {
                    log_warn!("não foi possível trocar o monitor para {label}: {e}");
                    (false, e)
                }
            };
            *PC.lock().unwrap() = Some(result);
        }))
    });
    let _ = worker.lock().unwrap().send(request);
}

pub fn set(choice: Choice) {
    *CHOICE.lock().unwrap() = Some(choice);
    cancel();
    *NOTEBOOK.lock().unwrap() = None;
    *PC.lock().unwrap() = None;
}

pub fn setup_message() -> Message {
    let choice = CHOICE.lock().unwrap().clone().unwrap_or_default();
    Message::MonitorSetup {
        enabled: choice.active(),
        pc_input: choice.pc_input,
        notebook_input: choice.notebook_input,
        delay_ms: choice.delay().as_millis() as u16,
        name: choice.name,
    }
}

pub fn show_notebook() {
    let choice = CHOICE.lock().unwrap().clone();
    if let Some(choice) = choice.filter(Choice::active) {
        request(Request::Switch {
            delay: choice.delay(),
            name: choice.name,
            value: choice.notebook_input,
        });
    }
}

pub fn set_tap(choice: TapChoice) {
    *TAP.lock().unwrap() = Some(choice);
}

pub fn tap_message() -> Message {
    let choice = TAP.lock().unwrap().clone().filter(TapChoice::active);
    match choice {
        Some(c) => Message::TapSetup {
            pc_input: c.pc_input,
            name: c.name,
        },
        None => Message::TapSetup {
            pc_input: 0,
            name: String::new(),
        },
    }
}

pub fn tap_name() -> Option<String> {
    TAP.lock()
        .unwrap()
        .as_ref()
        .map(|choice| choice.name.clone())
}

pub fn tap_enabled() -> bool {
    TAP.lock().unwrap().as_ref().is_some_and(TapChoice::active)
}

pub fn tap_toggle(notebook: Sender<Message>, follow: fn(bool)) {
    let Some(choice) = TAP.lock().unwrap().clone().filter(TapChoice::active) else {
        return;
    };
    if TAP_BUSY.swap(true, Ordering::SeqCst) {
        return;
    }
    thread::spawn(move || {
        let name = choice.name.clone();
        match tap_move(&choice, ddc_win::read_input(&name)) {
            TapMove::Local(value) => {
                let label = input_name(value);
                let result = match ddc_win::set_input(&name, value) {
                    Ok(()) => {
                        log_info!("duas batidas no topo: monitor {name} trocado para {label}");
                        follow(true);
                        (true, format!("duas batidas: trocou para {label}"))
                    }
                    Err(e) => {
                        log_warn!(
                            "duas batidas no topo: não foi possível trocar para {label}: {e}"
                        );
                        (false, e)
                    }
                };
                *PC.lock().unwrap() = Some(result);
            }
            TapMove::Notebook(value) => {
                log_info!(
                    "duas batidas no topo: pedindo ao notebook a troca de {name} para {}",
                    input_name(value)
                );
                let _ = notebook.send(Message::MonitorSwitch { value, name });
                follow(false);
            }
        }
        TAP_BUSY.store(false, Ordering::SeqCst);
    });
}

pub fn cancel() {
    if WORKER.get().is_some() {
        request(Request::Cancel);
    }
}

pub fn set_notebook_report(ok: bool, detail: String) {
    if !ok {
        log_warn!("notebook não consegue trocar o monitor: {detail}");
    }
    *NOTEBOOK.lock().unwrap() = Some((ok, detail));
}

pub fn notebook_report() -> Option<(bool, String)> {
    NOTEBOOK.lock().unwrap().clone()
}

pub fn pc_report() -> Option<(bool, String)> {
    PC.lock().unwrap().clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choice_needs_everything_to_be_active() {
        let full = Choice {
            enabled: true,
            name: "34GL750".into(),
            pc_input: 0x0F,
            notebook_input: 0x12,
            guard: true,
        };
        assert!(full.active());
        assert_eq!(full.delay(), SWITCH_DELAY);
        assert_eq!(
            Choice {
                guard: false,
                ..full.clone()
            }
            .delay(),
            Duration::ZERO
        );
        assert!(!Choice {
            enabled: false,
            ..full.clone()
        }
        .active());
        assert!(!Choice {
            name: String::new(),
            ..full.clone()
        }
        .active());
        assert!(!Choice {
            notebook_input: 0,
            ..full
        }
        .active());
    }

    #[test]
    fn tap_toggles_from_wherever_the_monitor_is() {
        let tap = TapChoice {
            enabled: true,
            name: "U2723QE".into(),
            pc_input: 0x0F,
            notebook_input: 0x11,
        };
        assert!(tap.active());
        assert_eq!(tap_move(&tap, Some(0x0F)), TapMove::Local(0x11));
        assert_eq!(tap_move(&tap, Some(0x10)), TapMove::Local(0x11));
        assert_eq!(tap_move(&tap, None), TapMove::Notebook(0x0F));
        assert_eq!(tap_move(&tap, Some(0x11)), TapMove::Notebook(0x0F));
        assert!(!TapChoice {
            pc_input: 0,
            ..tap.clone()
        }
        .active());
        assert!(!TapChoice {
            enabled: false,
            ..tap
        }
        .active());
    }
}
