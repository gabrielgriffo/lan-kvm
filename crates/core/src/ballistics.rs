pub const DEFAULT_CURVE_X: [f64; 5] = [0.0, 28181.0 / 65536.0, 1.25, 252969.0 / 65536.0, 40.0];
pub const DEFAULT_CURVE_Y: [f64; 5] = [0.0, 70141.0 / 65536.0, 4.140625, 18.984375, 443.75];

const SPEED_DIVISOR: f64 = 3.5;
const MAX_BATCH: i32 = 64;
const MAX_DEBT: i32 = -4;
const DPI_DIVISOR: f64 = 120.0;
const LINEAR_FACTORS: [f64; 20] = [
    0.03125, 0.0625, 0.125, 0.25, 0.375, 0.5, 0.625, 0.75, 0.875, 1.0, 1.25, 1.5, 1.75, 2.0, 2.25,
    2.5, 2.75, 3.0, 3.25, 3.5,
];

#[derive(Debug, Clone, PartialEq)]
enum Mode {
    Linear(f64),
    Enhanced {
        x: [f64; 5],
        y: [f64; 5],
        scale: f64,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Ballistics {
    mode: Mode,
    rest: (f64, f64),
}

impl Default for Ballistics {
    fn default() -> Self {
        Self::linear(10)
    }
}

fn clamp_speed(speed: u32) -> u32 {
    speed.clamp(1, 20)
}

impl Ballistics {
    pub fn linear(speed: u32) -> Self {
        Self {
            mode: Mode::Linear(LINEAR_FACTORS[clamp_speed(speed) as usize - 1]),
            rest: (0.0, 0.0),
        }
    }

    pub fn enhanced(speed: u32, x: [f64; 5], y: [f64; 5], dpi: u32) -> Self {
        let scale = dpi as f64 / DPI_DIVISOR * clamp_speed(speed) as f64 / 10.0;
        Self {
            mode: Mode::Enhanced { x, y, scale },
            rest: (0.0, 0.0),
        }
    }

    pub fn reset(&mut self) {
        self.rest = (0.0, 0.0);
    }

    fn gain(&self, dx: i32, dy: i32, reports: u32) -> f64 {
        match &self.mode {
            Mode::Linear(factor) => *factor,
            Mode::Enhanced { x, y, scale } => {
                let (ax, ay) = (dx.unsigned_abs() as f64, dy.unsigned_abs() as f64);
                let magnitude = (ax.max(ay) + ax.min(ay) / 2.0) / f64::from(reports.max(1));
                if magnitude == 0.0 {
                    return 0.0;
                }
                scale * interpolate(x, y, magnitude / SPEED_DIVISOR) / magnitude
            }
        }
    }

    pub fn apply(&mut self, dx: i32, dy: i32) -> (i32, i32) {
        self.apply_batched(dx, dy, 1)
    }

    pub fn apply_batched(&mut self, dx: i32, dy: i32, reports: u32) -> (i32, i32) {
        let gain = self.gain(dx, dy, reports);
        let tx = self.rest.0 + dx as f64 * gain;
        let ty = self.rest.1 + dy as f64 * gain;
        let (ox, oy) = (tx.trunc(), ty.trunc());
        self.rest = (tx - ox, ty - oy);
        (ox as i32, oy as i32)
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Coalescing {
    pending: i32,
}

impl Coalescing {
    pub fn report(&mut self) {
        self.pending = (self.pending + 1).min(MAX_BATCH);
    }

    pub fn take(&mut self) -> u32 {
        let reports = self.pending.max(1);
        self.pending = (self.pending - reports).max(MAX_DEBT);
        reports as u32
    }

    pub fn reset(&mut self) {
        self.pending = 0;
    }
}

fn interpolate(xs: &[f64; 5], ys: &[f64; 5], x: f64) -> f64 {
    let last = xs.len() - 1;
    let i = (1..=last).find(|&i| x <= xs[i]).unwrap_or(last);
    let span = xs[i] - xs[i - 1];
    if span <= 0.0 {
        return ys[i];
    }
    ys[i - 1] + (ys[i] - ys[i - 1]) * (x - xs[i - 1]) / span
}

pub fn parse_curve(bytes: &[u8]) -> Option<[f64; 5]> {
    if bytes.len() < 40 {
        return None;
    }
    let mut out = [0.0; 5];
    for (i, value) in out.iter_mut().enumerate() {
        let raw = u32::from_le_bytes(bytes[i * 8..i * 8 + 4].try_into().ok()?);
        *value = raw as f64 / 65536.0;
    }
    out.windows(2).all(|w| w[0] <= w[1]).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn moved(b: &mut Ballistics, packet: i32, count: usize) -> i32 {
        (0..count).map(|_| b.apply(packet, 0).0).sum()
    }

    fn windows_default(speed: u32) -> Ballistics {
        Ballistics::enhanced(speed, DEFAULT_CURVE_X, DEFAULT_CURVE_Y, 96)
    }

    #[test]
    fn a_coalesced_packet_moves_like_the_reports_it_carries() {
        for packet in [1, 3, 5, 12] {
            for reports in [2, 7, 8] {
                let separate = moved(&mut windows_default(10), packet, reports as usize);
                let batched = windows_default(10)
                    .apply_batched(packet * reports, 0, reports as u32)
                    .0;
                assert!(
                    (batched - separate).abs() <= 1,
                    "pacote {packet} x {reports}: {batched} vs {separate}"
                );
            }
        }
        let mut linear = Ballistics::linear(10);
        assert_eq!(linear.apply_batched(40, 0, 8), (40, 0));
    }

    #[test]
    fn coalescing_counts_hook_reports_between_raw_events() {
        let mut c = Coalescing::default();
        for _ in 0..5 {
            c.report();
            assert_eq!(c.take(), 1);
        }
        for _ in 0..5 {
            assert_eq!(c.take(), 1);
            c.report();
        }
        for _ in 0..7 {
            c.report();
        }
        assert_eq!(c.take(), 7);
        for _ in 0..20 {
            assert_eq!(c.take(), 1);
        }
        for _ in 0..3 {
            c.report();
        }
        assert_eq!(c.take(), 1);
        for _ in 0..100 {
            c.report();
        }
        assert_eq!(c.take(), 64);
        c.report();
        c.reset();
        assert_eq!(c.take(), 1);
    }

    #[test]
    fn enhanced_matches_windows_measurements() {
        let measured = [
            (1, 600, 340),
            (2, 300, 384),
            (4, 150, 449),
            (6, 100, 542),
            (8, 75, 602),
            (12, 50, 661),
            (16, 38, 832),
            (20, 30, 978),
            (30, 20, 1189),
            (40, 15, 1296),
            (80, 8, 1550),
        ];
        for (packet, count, pixels) in measured {
            let got = moved(&mut windows_default(10), packet, count);
            assert!(
                (got - pixels).abs() <= 2,
                "pacote {packet}: {got} vs {pixels}"
            );
        }
    }

    #[test]
    fn speed_slider_scales_enhanced_linearly() {
        for (speed, packet, count, pixels) in
            [(6, 40, 10, 518), (6, 1, 400, 136), (14, 80, 6, 1628)]
        {
            let got = moved(&mut windows_default(speed), packet, count);
            assert!(
                (got - pixels).abs() <= 2,
                "velocidade {speed}: {got} vs {pixels}"
            );
        }
    }

    #[test]
    fn linear_uses_the_classic_table_and_keeps_fractions() {
        assert_eq!(moved(&mut Ballistics::linear(10), 3, 100), 300);
        assert_eq!(moved(&mut Ballistics::linear(6), 1, 400), 200);
        assert_eq!(moved(&mut Ballistics::linear(1), 1, 64), 2);
        assert_eq!(moved(&mut Ballistics::linear(20), 2, 10), 70);
        assert_eq!(moved(&mut Ballistics::linear(99), 2, 10), 70);
    }

    #[test]
    fn direction_and_diagonals_are_preserved() {
        let mut b = windows_default(10);
        let (x, y) = b.apply(-40, -40);
        assert!(x < 0 && y < 0 && x == y);
        let mut b = windows_default(10);
        assert_eq!(b.apply(0, 0), (0, 0));
        let (x, y) = b.apply(0, 30);
        assert_eq!(x, 0);
        assert!(y > 30);
    }

    #[test]
    fn reset_drops_the_fraction() {
        let mut b = Ballistics::linear(6);
        assert_eq!(b.apply(1, 0), (0, 0));
        b.reset();
        assert_eq!(b.apply(1, 0), (0, 0));
        assert_eq!(b.apply(1, 0), (1, 0));
    }

    #[test]
    fn parses_registry_curves() {
        let x: [u8; 40] = [
            0, 0, 0, 0, 0, 0, 0, 0, 0x15, 0x6E, 0, 0, 0, 0, 0, 0, 0, 0x40, 0x01, 0, 0, 0, 0, 0,
            0x29, 0xDC, 0x03, 0, 0, 0, 0, 0, 0, 0, 0x28, 0, 0, 0, 0, 0,
        ];
        let curve = parse_curve(&x).unwrap();
        for (got, want) in curve.iter().zip(DEFAULT_CURVE_X) {
            assert!((got - want).abs() < 0.001, "{got} vs {want}");
        }
        assert_eq!(parse_curve(&x[..39]), None);
        let mut decreasing = x;
        decreasing[8..16].copy_from_slice(&[0, 0, 0x50, 0, 0, 0, 0, 0]);
        assert_eq!(parse_curve(&decreasing), None);
    }
}
