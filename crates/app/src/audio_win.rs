use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream, StreamConfig};
use lan_kvm_core::audio::{
    decode, encode, frame_samples, remix, Jitter, Resampler, MIC_CHANNELS, RATE, SPEAKER_CHANNELS,
};
use lan_kvm_core::proto::Message;

const START_MS: u32 = 60;
const MAX_MS: u32 = 250;
const OPEN_TIMEOUT: Duration = Duration::from_secs(10);

struct Playback {
    jitter: Jitter,
    resampler: Resampler,
    channels: usize,
}

struct Running {
    stop: Sender<()>,
}

static RUNNING: Mutex<Option<Running>> = Mutex::new(None);
static GENERATION: AtomicU64 = AtomicU64::new(0);
static PLAYBACK: Mutex<Option<Playback>> = Mutex::new(None);
static REPORT: Mutex<Option<(bool, String)>> = Mutex::new(None);
static NOTEBOOK: Mutex<Option<(bool, String)>> = Mutex::new(None);

pub fn pc_report() -> Option<(bool, String)> {
    REPORT.lock().unwrap().clone()
}

pub fn notebook_report() -> Option<(bool, String)> {
    NOTEBOOK.lock().unwrap().clone()
}

pub fn set_notebook_report(ok: bool, detail: String) {
    if !ok {
        log_warn!("áudio no notebook: {detail}");
    }
    *NOTEBOOK.lock().unwrap() = Some((ok, detail));
}

pub fn play(pcm: &[u8]) {
    let mut guard = PLAYBACK.lock().unwrap();
    let Some(p) = guard.as_mut() else {
        return;
    };
    let samples = p.resampler.process(&decode(pcm));
    let samples = remix(&samples, SPEAKER_CHANNELS, p.channels);
    p.jitter.push(&samples);
}

fn as_f32<T: Copy>(data: &[T], convert: fn(T) -> f32) -> Vec<f32> {
    data.iter().map(|&s| convert(s)).collect()
}

fn i16_to_f32(s: i16) -> f32 {
    s as f32 / 32768.0
}

fn open_mic(tx: Sender<Message>) -> Result<Stream, String> {
    let device = cpal::default_host()
        .default_input_device()
        .ok_or("nenhum microfone padrão no Windows")?;
    let supported = device
        .default_input_config()
        .map_err(|e| format!("microfone: {e}"))?;
    let format = supported.sample_format();
    let config: StreamConfig = supported.into();
    let channels = config.channels as usize;
    let resampler = Mutex::new(Resampler::new(config.sample_rate.0, RATE, MIC_CHANNELS));
    let pending = Mutex::new(Vec::<f32>::new());
    let chunk = frame_samples(MIC_CHANNELS);
    let deliver = move |data: Vec<f32>| {
        let mono = remix(&data, channels, MIC_CHANNELS);
        let resampled = resampler.lock().unwrap().process(&mono);
        let mut pending = pending.lock().unwrap();
        pending.extend(resampled);
        while pending.len() >= chunk {
            let frame: Vec<f32> = pending.drain(..chunk).collect();
            let _ = tx.send(Message::Mic(encode(&frame)));
        }
    };
    let error = |e| log_warn!("erro no microfone: {e}");
    let stream = match format {
        SampleFormat::F32 => device.build_input_stream(
            &config,
            move |data: &[f32], _| deliver(data.to_vec()),
            error,
            None,
        ),
        SampleFormat::I16 => device.build_input_stream(
            &config,
            move |data: &[i16], _| deliver(as_f32(data, i16_to_f32)),
            error,
            None,
        ),
        other => return Err(format!("formato de microfone {other:?} não suportado")),
    }
    .map_err(|e| format!("microfone: {e}"))?;
    stream.play().map_err(|e| format!("microfone: {e}"))?;
    Ok(stream)
}

fn open_speaker() -> Result<(Stream, String), String> {
    let device = cpal::default_host()
        .default_output_device()
        .ok_or("nenhuma saída de som padrão no Windows")?;
    let name = device.name().unwrap_or_else(|_| "saída padrão".to_string());
    let supported = device
        .default_output_config()
        .map_err(|e| format!("saída de som: {e}"))?;
    let format = supported.sample_format();
    let config: StreamConfig = supported.into();
    let channels = config.channels as usize;
    *PLAYBACK.lock().unwrap() = Some(Playback {
        jitter: Jitter::new(config.sample_rate.0, channels, START_MS, MAX_MS),
        resampler: Resampler::new(RATE, config.sample_rate.0, SPEAKER_CHANNELS),
        channels,
    });
    let fill = |out: &mut [f32]| match PLAYBACK.lock().unwrap().as_mut() {
        Some(p) => p.jitter.pop_into(out),
        None => out.fill(0.0),
    };
    let error = |e| log_warn!("erro na saída de som: {e}");
    let stream = match format {
        SampleFormat::F32 => {
            device.build_output_stream(&config, move |out: &mut [f32], _| fill(out), error, None)
        }
        SampleFormat::I16 => device.build_output_stream(
            &config,
            move |out: &mut [i16], _| {
                let mut buf = vec![0.0; out.len()];
                fill(&mut buf);
                for (o, s) in out.iter_mut().zip(buf) {
                    *o = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
                }
            },
            error,
            None,
        ),
        other => return Err(format!("formato de saída {other:?} não suportado")),
    }
    .map_err(|e| format!("saída de som: {e}"))?;
    stream.play().map_err(|e| format!("saída de som: {e}"))?;
    Ok((stream, name))
}

fn set_report(generation: u64, report: Result<String, String>) {
    if GENERATION.load(Ordering::SeqCst) != generation {
        return;
    }
    match &report {
        Ok(detail) => log_info!("áudio do notebook ligado: {detail}"),
        Err(e) => log_warn!("áudio do notebook: {e}"),
    }
    *REPORT.lock().unwrap() = Some(match report {
        Ok(detail) => (true, detail),
        Err(e) => (false, e),
    });
}

pub fn start(tx: Sender<Message>) {
    stop();
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let (ready_tx, ready_rx) = mpsc::channel::<Result<String, String>>();
    thread::spawn(move || {
        let speaker = open_speaker();
        let mic = open_mic(tx);
        let result = match (&speaker, &mic) {
            (Ok((_, name)), Ok(_)) => Ok(format!("som do notebook em {name}, microfone enviado")),
            (Err(e), _) | (_, Err(e)) => Err(e.clone()),
        };
        let _ = ready_tx.send(result);
        let _ = stop_rx.recv();
        drop(mic);
        drop(speaker);
        if GENERATION.load(Ordering::SeqCst) == generation {
            *PLAYBACK.lock().unwrap() = None;
        }
    });
    thread::spawn(move || {
        let report = match ready_rx.recv_timeout(OPEN_TIMEOUT) {
            Err(RecvTimeoutError::Timeout) => {
                let waiting = format!(
                    "o Windows não abriu o microfone ou a saída de som em {} s; se um \
                     antivírus pedir permissão para o Lan Kvm usar o microfone, aprove \
                     e o áudio liga sozinho",
                    OPEN_TIMEOUT.as_secs()
                );
                set_report(generation, Err(waiting));
                ready_rx
                    .recv()
                    .unwrap_or_else(|_| Err("a abertura do áudio falhou".to_string()))
            }
            Ok(report) => report,
            Err(RecvTimeoutError::Disconnected) => Err("a abertura do áudio falhou".to_string()),
        };
        set_report(generation, report);
    });
    *RUNNING.lock().unwrap() = Some(Running { stop: stop_tx });
}

pub fn stop() {
    let running = RUNNING.lock().unwrap().take();
    if let Some(running) = running {
        GENERATION.fetch_add(1, Ordering::SeqCst);
        let _ = running.stop.send(());
        *PLAYBACK.lock().unwrap() = None;
        log_info!("áudio do notebook desligado");
    }
}

pub fn clear_reports() {
    *REPORT.lock().unwrap() = None;
    *NOTEBOOK.lock().unwrap() = None;
}
