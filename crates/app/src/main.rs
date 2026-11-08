#![cfg_attr(windows, windows_subsystem = "windows")]

#[macro_use]
mod logging;

#[cfg(target_os = "linux")]
mod audio_linux;
#[cfg(windows)]
mod audio_win;
#[cfg(windows)]
mod autostart;
#[cfg(target_os = "linux")]
mod capture;
#[cfg(target_os = "linux")]
mod ddc_linux;
#[cfg(windows)]
mod ddc_win;
#[cfg(windows)]
mod gui;
#[cfg(windows)]
mod inject;
mod keystore;
#[cfg(windows)]
mod monitor;
#[cfg(windows)]
mod mouse_speed;
mod net;
#[cfg(windows)]
mod pointer;
#[cfg(windows)]
mod win;
#[cfg(target_os = "linux")]
mod x11;

use std::error::Error;

use net::{with_port, DEFAULT_PORT};

const USAGE: &str = "uso:
  lan-kvm                                abre a interface (Windows)
  lan-kvm send <host[:porta]> [--left] [--protect-corners] [--monitor NOME,PC,NOTEBOOK [--monitor-now]]
                                         captura mouse e teclado sem interface (Windows)
  lan-kvm pair <host[:porta]>            pareia com o notebook (Windows)
  lan-kvm recv [endereço:porta]          recebe mouse e teclado (Linux X11)
  lan-kvm unpair                         remove o pareamento desta máquina
  lan-kvm monitors                       lista monitores e entradas por DDC/CI
  lan-kvm --version                      mostra a versão";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    logging::init();
    #[cfg(windows)]
    if let [] | ["--tray"] = args.as_slice() {
        log_info!(
            "lan-kvm {} iniciado{}",
            lan_kvm_core::VERSION,
            if args.is_empty() { "" } else { " na bandeja" }
        );
        let result = single_instance().and_then(|instance| match instance {
            Instance::First => gui::run(!args.is_empty()),
            Instance::Running => {
                log_info!("já estava aberto: pedido à janela existente para aparecer");
                gui::show_running();
                Ok(())
            }
        });
        if let Err(e) = result {
            log_error!("{e}");
            logging::flush();
            win_alert(&e.to_string());
            std::process::exit(1);
        }
        log_info!("encerrado");
        logging::flush();
        return;
    }
    #[cfg(windows)]
    attach_console();
    let result: Result<(), Box<dyn Error>> = match args.as_slice() {
        ["send", addr, rest @ ..] => send(&with_port(addr), rest),
        ["recv"] => recv(&format!("0.0.0.0:{DEFAULT_PORT}")),
        ["recv", bind] => recv(&with_port(bind)),
        ["unpair"] => unpair(),
        ["monitors"] => monitors(),
        ["pair", addr] => pair(&with_port(addr)),
        ["--version" | "-V"] => {
            println!("lan-kvm {}", lan_kvm_core::VERSION);
            Ok(())
        }
        _ => {
            println!(
                "lan-kvm {} ({})\n{USAGE}",
                lan_kvm_core::VERSION,
                std::env::consts::OS
            );
            Ok(())
        }
    };
    if let Err(e) = result {
        log_error!("{e}");
        logging::flush();
        std::process::exit(1);
    }
}

#[cfg(windows)]
fn attach_console() {
    use windows_sys::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
    unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
}

#[cfg(windows)]
enum Instance {
    First,
    Running,
}

#[cfg(windows)]
fn single_instance() -> Result<Instance, Box<dyn Error>> {
    use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows_sys::Win32::System::Threading::CreateMutexW;
    let name: Vec<u16> = "Local\\lan-kvm-sender"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    if handle.is_null() {
        return Err(std::io::Error::last_os_error().into());
    }
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        return Ok(Instance::Running);
    }
    Ok(Instance::First)
}

#[cfg(windows)]
fn win_alert(text: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
    let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(Some(0)).collect() };
    let (text, title) = (wide(text), wide("Lan Kvm"));
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        )
    };
}

#[cfg(windows)]
fn send(addr: &str, flags: &[&str]) -> Result<(), Box<dyn Error>> {
    use lan_kvm_core::switch::Side;
    if let Instance::Running = single_instance()? {
        return Err("o Lan Kvm já está em execução (veja o ícone na bandeja)".into());
    }
    let left = flags.contains(&"--left");
    win::set_corner_guard(flags.contains(&"--protect-corners"));
    if let Some(spec) = flags.windows(2).find(|w| w[0] == "--monitor").map(|w| w[1]) {
        let parts: Vec<&str> = spec.split(',').collect();
        let byte = |s: &str| u8::from_str_radix(s.trim_start_matches("0x"), 16).ok();
        match parts.as_slice() {
            [name, pc, nb] => win::set_monitor(monitor::Choice {
                enabled: true,
                name: name.to_string(),
                pc_input: byte(pc).ok_or("entrada do PC inválida")?,
                notebook_input: byte(nb).ok_or("entrada do notebook inválida")?,
                guard: !flags.contains(&"--monitor-now"),
            }),
            _ => return Err("use --monitor NOME,PC,NOTEBOOK (entradas em hexadecimal)".into()),
        }
    }
    win::run_sender(addr, if left { Side::Left } else { Side::Right })
}

#[cfg(not(windows))]
fn send(_: &str, _: &[&str]) -> Result<(), Box<dyn Error>> {
    Err("send só está disponível no Windows".into())
}

#[cfg(windows)]
fn pair(addr: &str) -> Result<(), Box<dyn Error>> {
    use std::io::{BufRead, Write};
    println!("pedindo um código ao notebook {addr}...");
    let stream = win::pair_request(addr)?;
    print!("digite o código de 6 dígitos que apareceu no notebook: ");
    std::io::stdout().flush()?;
    let mut code = String::new();
    if std::io::stdin().lock().read_line(&mut code).unwrap_or(0) == 0 {
        let console = std::fs::File::open("CONIN$")?;
        std::io::BufReader::new(console).read_line(&mut code)?;
    }
    win::pair_complete(stream, &code)?;
    println!("pareado com sucesso");
    Ok(())
}

#[cfg(not(windows))]
fn pair(_: &str) -> Result<(), Box<dyn Error>> {
    Err("o pareamento é iniciado pelo PC com Windows".into())
}

#[cfg(windows)]
fn monitors() -> Result<(), Box<dyn Error>> {
    use lan_kvm_core::ddc::input_name;
    let list = ddc_win::list();
    if list.is_empty() {
        println!("nenhum monitor respondeu por DDC/CI");
    }
    for m in list {
        let inputs: Vec<String> = m
            .inputs
            .iter()
            .map(|&v| format!("{} (0x{v:02X})", input_name(v)))
            .collect();
        let current = m.current.map_or("desconhecida".to_string(), |v| {
            format!("{} (0x{v:02X})", input_name(v))
        });
        println!(
            "{}: entrada atual {current}; entradas: {}",
            m.name,
            inputs.join(", ")
        );
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn monitors() -> Result<(), Box<dyn Error>> {
    let list = ddc_linux::displays();
    if list.is_empty() {
        println!("nenhum monitor conectado com EDID legível");
    }
    for d in list {
        let access = match ddc_linux::check(&d.name) {
            Ok(_) => "acesso DDC ok".to_string(),
            Err(e) => e,
        };
        println!("{} em {}: {access}", d.name, d.connector);
    }
    Ok(())
}

fn unpair() -> Result<(), Box<dyn Error>> {
    #[cfg(target_os = "linux")]
    return x11::unpair();
    #[cfg(not(target_os = "linux"))]
    {
        keystore::forget()?;
        println!("pareamento removido deste PC");
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn recv(bind: &str) -> Result<(), Box<dyn Error>> {
    x11::run_receiver(bind)
}

#[cfg(not(target_os = "linux"))]
fn recv(_: &str) -> Result<(), Box<dyn Error>> {
    Err("recv só está disponível no Linux".into())
}
