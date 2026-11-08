use std::io::{self, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;

use lan_kvm_core::audio::{frame_samples, MIC_CHANNELS, RATE, SPEAKER_CHANNELS};
use lan_kvm_core::proto::Message;

const SINK: &str = "lan_kvm_fone";
const MIC_SINK: &str = "lan_kvm_mic_entrada";
const MIC: &str = "lan_kvm_mic";
const MIC_QUEUE: usize = 25;

pub type Outbox = Arc<dyn Fn(&Message) -> io::Result<()> + Send + Sync>;

fn pactl(args: &[&str]) -> Result<String, String> {
    let out = Command::new("pactl")
        .args(args)
        .env("LANG", "C")
        .output()
        .map_err(|e| format!("pactl indisponível: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "pactl {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn description(key: &str, text: &str) -> String {
    format!("{key}=\"device.description='{text}'\"")
}

fn ids(kind: &str) -> Vec<String> {
    pactl(&["list", "short", kind])
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.split('\t').next().map(str::to_string))
        .collect()
}

fn our_modules() -> Vec<String> {
    pactl(&["list", "short", "modules"])
        .unwrap_or_default()
        .lines()
        .filter(|line| line.contains("lan_kvm_"))
        .filter_map(|line| line.split('\t').next().map(str::to_string))
        .collect()
}

fn audio_args(channels: usize) -> Vec<String> {
    vec![
        "--raw".to_string(),
        "--format=s16le".to_string(),
        format!("--rate={RATE}"),
        format!("--channels={channels}"),
        "--latency-msec=20".to_string(),
    ]
}

struct Active {
    modules: Vec<String>,
    previous: (String, String),
    capture: Child,
    mic: Child,
    mic_tx: SyncSender<Vec<u8>>,
}

pub struct AudioBridge {
    active: Mutex<Option<Active>>,
}

impl AudioBridge {
    pub fn new() -> Arc<Self> {
        for id in our_modules().into_iter().rev() {
            let _ = pactl(&["unload-module", &id]);
        }
        Arc::new(Self {
            active: Mutex::new(None),
        })
    }

    pub fn set(&self, enabled: bool, outbox: Outbox) -> Option<Message> {
        self.disable();
        if !enabled {
            return None;
        }
        Some(match self.enable(outbox) {
            Ok(detail) => {
                log_info!("áudio do PC ligado: {detail}");
                Message::AudioReport { ok: true, detail }
            }
            Err(detail) => {
                log_warn!("áudio do PC: {detail}");
                Message::AudioReport { ok: false, detail }
            }
        })
    }

    fn enable(&self, outbox: Outbox) -> Result<String, String> {
        let previous = (
            pactl(&["get-default-sink"]).unwrap_or_default(),
            pactl(&["get-default-source"]).unwrap_or_default(),
        );
        let mut modules = Vec::new();
        let result = Self::create(&mut modules).and_then(|()| {
            pactl(&["set-default-sink", SINK])?;
            pactl(&["set-default-source", MIC])?;
            for id in ids("sink-inputs") {
                let _ = pactl(&["move-sink-input", &id, SINK]);
            }
            for id in ids("source-outputs") {
                let _ = pactl(&["move-source-output", &id, MIC]);
            }
            Self::spawn(outbox)
        });
        match result {
            Ok((capture, mic, mic_tx)) => {
                *self.active.lock().unwrap() = Some(Active {
                    modules,
                    previous,
                    capture,
                    mic,
                    mic_tx,
                });
                Ok(
                    "saída \"lan-kvm: fone do PC\" e microfone \"lan-kvm: microfone do PC\" \
                    em uso como padrão"
                        .to_string(),
                )
            }
            Err(e) => {
                restore(&previous);
                for id in modules.iter().rev() {
                    let _ = pactl(&["unload-module", id]);
                }
                Err(e)
            }
        }
    }

    fn create(modules: &mut Vec<String>) -> Result<(), String> {
        let rate = format!("rate={RATE}");
        let mut load = |args: Vec<String>| -> Result<(), String> {
            let mut all = vec!["load-module".to_string()];
            all.extend(args);
            let refs: Vec<&str> = all.iter().map(String::as_str).collect();
            modules.push(pactl(&refs)?);
            Ok(())
        };
        load(vec![
            "module-null-sink".into(),
            format!("sink_name={SINK}"),
            rate.clone(),
            format!("channels={SPEAKER_CHANNELS}"),
            description("sink_properties", "lan-kvm: fone do PC"),
        ])?;
        load(vec![
            "module-null-sink".into(),
            format!("sink_name={MIC_SINK}"),
            rate,
            format!("channels={MIC_CHANNELS}"),
            description("sink_properties", "lan-kvm: entrada interna"),
        ])?;
        load(vec![
            "module-remap-source".into(),
            format!("source_name={MIC}"),
            format!("master={MIC_SINK}.monitor"),
            description("source_properties", "lan-kvm: microfone do PC"),
        ])
    }

    fn spawn(outbox: Outbox) -> Result<(Child, Child, SyncSender<Vec<u8>>), String> {
        let mut capture = Command::new("parec")
            .args(audio_args(SPEAKER_CHANNELS))
            .arg(format!("--device={SINK}.monitor"))
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("parec indisponível: {e}"))?;
        let mut mic = match Command::new("pacat")
            .arg("--playback")
            .args(audio_args(MIC_CHANNELS))
            .arg(format!("--device={MIC_SINK}"))
            .stdin(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(e) => {
                let _ = capture.kill();
                let _ = capture.wait();
                return Err(format!("pacat indisponível: {e}"));
            }
        };
        let mut stdout = capture.stdout.take().ok_or("parec sem saída")?;
        thread::spawn(move || {
            let mut frame = vec![0u8; frame_samples(SPEAKER_CHANNELS) * 2];
            while stdout.read_exact(&mut frame).is_ok() {
                if frame.iter().all(|&b| b == 0) {
                    continue;
                }
                if outbox(&Message::Speaker(frame.clone())).is_err() {
                    break;
                }
            }
        });
        let mut stdin = mic.stdin.take().ok_or("pacat sem entrada")?;
        let (mic_tx, mic_rx) = mpsc::sync_channel::<Vec<u8>>(MIC_QUEUE);
        thread::spawn(move || {
            for pcm in mic_rx {
                if stdin.write_all(&pcm).is_err() {
                    break;
                }
            }
        });
        Ok((capture, mic, mic_tx))
    }

    pub fn mic(&self, pcm: Vec<u8>) {
        if let Some(active) = self.active.lock().unwrap().as_ref() {
            let _ = active.mic_tx.try_send(pcm);
        }
    }

    pub fn disable(&self) {
        let Some(mut active) = self.active.lock().unwrap().take() else {
            return;
        };
        drop(active.mic_tx);
        for child in [&mut active.capture, &mut active.mic] {
            let _ = child.kill();
            let _ = child.wait();
        }
        restore(&active.previous);
        for id in active.modules.iter().rev() {
            let _ = pactl(&["unload-module", id]);
        }
        log_info!("áudio do PC desligado");
    }
}

fn restore((sink, source): &(String, String)) {
    if !sink.is_empty() && sink != SINK {
        let _ = pactl(&["set-default-sink", sink]);
    }
    if !source.is_empty() && source != MIC {
        let _ = pactl(&["set-default-source", source]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn description_survives_pulseaudio_quoting() {
        assert_eq!(
            description("sink_properties", "lan-kvm: fone do PC"),
            "sink_properties=\"device.description='lan-kvm: fone do PC'\""
        );
    }

    #[test]
    fn audio_args_describe_raw_48k_pcm() {
        assert_eq!(
            audio_args(2),
            [
                "--raw",
                "--format=s16le",
                "--rate=48000",
                "--channels=2",
                "--latency-msec=20"
            ]
        );
    }
}
