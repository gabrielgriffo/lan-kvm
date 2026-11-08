use std::collections::VecDeque;

pub const RATE: u32 = 48_000;
pub const SPEAKER_CHANNELS: usize = 2;
pub const MIC_CHANNELS: usize = 1;
pub const FRAME_MS: u32 = 20;

pub const fn frame_samples(channels: usize) -> usize {
    (RATE * FRAME_MS / 1000) as usize * channels
}

pub fn encode(samples: &[f32]) -> Vec<u8> {
    samples
        .iter()
        .flat_map(|s| ((s.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes())
        .collect()
}

pub fn decode(bytes: &[u8]) -> Vec<f32> {
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&b| i16::from_le_bytes(b) as f32 / 32768.0)
        .collect()
}

pub fn remix(input: &[f32], from: usize, to: usize) -> Vec<f32> {
    if from == to || from == 0 || to == 0 {
        return input.to_vec();
    }
    let mut out = Vec::with_capacity(input.len() / from * to);
    for frame in input.chunks_exact(from) {
        if to == 1 {
            out.push(frame.iter().sum::<f32>() / from as f32);
        } else {
            out.extend((0..to).map(|c| frame[c % from]));
        }
    }
    out
}

pub struct Resampler {
    channels: usize,
    step: f64,
    pos: f64,
    prev: Vec<f32>,
}

impl Resampler {
    pub fn new(from: u32, to: u32, channels: usize) -> Self {
        Self {
            channels: channels.max(1),
            step: from.max(1) as f64 / to.max(1) as f64,
            pos: 0.0,
            prev: vec![0.0; channels.max(1)],
        }
    }

    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        let c = self.channels;
        let n = input.len() / c;
        if n == 0 {
            return Vec::new();
        }
        let at = |i: isize, ch: usize| {
            if i < 0 {
                self.prev[ch]
            } else {
                input[i as usize * c + ch]
            }
        };
        let mut out = Vec::with_capacity(((n as f64 / self.step) as usize + 1) * c);
        while self.pos < n as f64 - 1.0 {
            let base = self.pos.floor();
            let frac = (self.pos - base) as f32;
            let i = base as isize;
            for ch in 0..c {
                let (a, b) = (at(i, ch), at(i + 1, ch));
                out.push(a + (b - a) * frac);
            }
            self.pos += self.step;
        }
        self.pos -= n as f64;
        self.prev.copy_from_slice(&input[(n - 1) * c..n * c]);
        out
    }
}

pub struct Jitter {
    buf: VecDeque<f32>,
    channels: usize,
    start: usize,
    max: usize,
    playing: bool,
}

impl Jitter {
    pub fn new(rate: u32, channels: usize, start_ms: u32, max_ms: u32) -> Self {
        let samples = |ms: u32| (rate as usize * ms as usize / 1000) * channels.max(1);
        Self {
            buf: VecDeque::new(),
            channels: channels.max(1),
            start: samples(start_ms),
            max: samples(max_ms),
            playing: false,
        }
    }

    pub fn buffered(&self) -> usize {
        self.buf.len()
    }

    pub fn push(&mut self, samples: &[f32]) {
        self.buf.extend(samples);
        if self.buf.len() > self.max {
            let excess = self.buf.len() - self.start;
            self.buf.drain(..excess - excess % self.channels);
        }
    }

    pub fn pop_into(&mut self, out: &mut [f32]) {
        if !self.playing && self.buf.len() >= self.start {
            self.playing = true;
        }
        let mut filled = 0;
        if self.playing {
            filled = out.len().min(self.buf.len());
            for (o, s) in out.iter_mut().zip(self.buf.drain(..filled)) {
                *o = s;
            }
            if filled < out.len() {
                self.playing = false;
            }
        }
        out[filled..].fill(0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm_roundtrip_keeps_values_and_clamps() {
        let samples = [0.0, 0.5, -0.5, 1.0, -1.0, 2.0];
        let back = decode(&encode(&samples));
        for (got, want) in back.iter().zip([0.0, 0.5, -0.5, 1.0, -1.0, 1.0]) {
            assert!((got - want).abs() < 0.001, "{got} vs {want}");
        }
        assert_eq!(encode(&[1.0]).len(), 2);
        assert_eq!(decode(&[1, 2, 3]).len(), 1);
    }

    #[test]
    fn remix_mixes_down_and_spreads_up() {
        assert_eq!(remix(&[0.2, 0.4, 1.0, 0.0], 2, 1), vec![0.3, 0.5]);
        assert_eq!(remix(&[0.1, 0.2], 1, 2), vec![0.1, 0.1, 0.2, 0.2]);
        assert_eq!(remix(&[0.1, 0.2], 2, 6), vec![0.1, 0.2, 0.1, 0.2, 0.1, 0.2]);
        assert_eq!(remix(&[0.1, 0.2], 2, 2), vec![0.1, 0.2]);
    }

    #[test]
    fn resampler_converts_rate_over_a_stream() {
        for (from, to) in [
            (44_100, 48_000),
            (48_000, 44_100),
            (16_000, 48_000),
            (48_000, 48_000),
        ] {
            let mut r = Resampler::new(from, to, 2);
            let mut total = 0;
            for _ in 0..50 {
                total += r.process(&vec![0.25; from as usize / 50 * 2]).len();
            }
            let expected = to as usize * 2;
            assert!(
                (total as i64 - expected as i64).abs() <= 8,
                "{from}->{to}: {total} vs {expected}"
            );
        }
    }

    #[test]
    fn resampler_keeps_a_constant_signal_after_the_first_frame() {
        let mut r = Resampler::new(44_100, 48_000, 1);
        let out = r.process(&[0.5; 441]);
        assert!(out[2..].iter().all(|s| (s - 0.5).abs() < 1e-6));
        let out = r.process(&[0.5; 441]);
        assert!(out.iter().all(|s| (s - 0.5).abs() < 1e-6));
    }

    #[test]
    fn resampler_interpolates_between_samples() {
        let mut r = Resampler::new(1, 2, 1);
        assert_eq!(r.process(&[0.0, 1.0, 0.0]), vec![0.0, 0.5, 1.0, 0.5]);
        assert_eq!(r.process(&[1.0]), vec![0.0, 0.5]);
    }

    #[test]
    fn jitter_waits_for_the_start_level_and_refills_after_running_dry() {
        let mut j = Jitter::new(1000, 1, 10, 50);
        let mut out = [9.0; 4];
        j.push(&[0.1; 8]);
        j.pop_into(&mut out);
        assert_eq!(out, [0.0; 4]);
        j.push(&[0.2; 2]);
        j.pop_into(&mut out);
        assert_eq!(out, [0.1; 4]);
        let mut big = [9.0; 8];
        j.pop_into(&mut big);
        assert_eq!(big, [0.1, 0.1, 0.1, 0.1, 0.2, 0.2, 0.0, 0.0]);
        j.push(&[0.3; 4]);
        j.pop_into(&mut out);
        assert_eq!(out, [0.0; 4]);
    }

    #[test]
    fn jitter_drops_old_audio_when_too_far_behind() {
        let mut j = Jitter::new(1000, 2, 10, 50);
        j.push(&[0.1; 120]);
        assert_eq!(j.buffered(), 20);
        let mut out = [9.0; 2];
        j.pop_into(&mut out);
        assert_eq!(out, [0.1; 2]);
        assert_eq!(frame_samples(SPEAKER_CHANNELS), 1920);
        assert_eq!(frame_samples(MIC_CHANNELS), 960);
    }
}
