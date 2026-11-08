use std::time::{Duration, Instant};

use crate::switch::Rect;

pub const DOUBLE_TAP_WINDOW: Duration = Duration::from_secs(1);
pub const REARM_DISTANCE: i32 = 8;
pub const REST_DISTANCE: i32 = 40;
pub const STILL_RADIUS: i32 = 12;
pub const STILL_TIME: Duration = Duration::from_millis(250);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LastSpot {
    rest: Option<(i32, i32)>,
    anchor: Option<Anchor>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Anchor {
    origin: (i32, i32),
    latest: (i32, i32),
    restful: bool,
    since: Instant,
}

impl LastSpot {
    pub fn observe(&mut self, position: (i32, i32), restful: bool, now: Instant) {
        if let Some(anchor) = self.anchor.as_mut() {
            let (ox, oy) = anchor.origin;
            if (position.0 - ox).abs() <= STILL_RADIUS && (position.1 - oy).abs() <= STILL_RADIUS {
                anchor.latest = position;
                return;
            }
            if anchor.restful && now.duration_since(anchor.since) >= STILL_TIME {
                self.rest = Some(anchor.latest);
            }
        }
        self.anchor = Some(Anchor {
            origin: position,
            latest: position,
            restful,
            since: now,
        });
    }

    pub fn get(&self) -> Option<(i32, i32)> {
        self.rest
    }
}

pub fn restful(top_distance: Option<i32>, taps: &TopTaps, now: Instant) -> bool {
    top_distance.is_none_or(|d| d >= REST_DISTANCE) && !taps.pending(now)
}

pub fn arrival(last: Option<(i32, i32)>, screen: Rect) -> (i32, i32) {
    last.filter(|&(x, y)| screen.contains(x, y))
        .unwrap_or_else(|| screen.center())
}

pub fn top_distance(monitors: &[Rect], px: i32, py: i32) -> Option<i32> {
    let m = monitors.iter().find(|m| m.contains(px, py.max(m.y)))?;
    let open = !monitors.iter().any(|o| o.contains(px, m.y - 1));
    open.then_some(py - m.y)
}

#[derive(Debug, Default)]
pub struct TopTaps {
    armed: bool,
    first: Option<Instant>,
}

impl TopTaps {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn pending(&self, now: Instant) -> bool {
        self.first
            .is_some_and(|first| now.duration_since(first) <= DOUBLE_TAP_WINDOW)
    }

    pub fn update(&mut self, distance: Option<i32>, now: Instant) -> bool {
        let Some(distance) = distance else {
            self.armed = false;
            return false;
        };
        if distance >= REARM_DISTANCE {
            self.armed = true;
            return false;
        }
        if distance > 0 || !self.armed {
            return false;
        }
        self.armed = false;
        match self.first.take() {
            Some(first) if now.duration_since(first) <= DOUBLE_TAP_WINDOW => true,
            _ => {
                self.first = Some(now);
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn walk(spot: &mut LastSpot, taps: &mut TopTaps, t0: Instant, steps: &[(u64, i32, i32)]) {
        for &(ms, x, y) in steps {
            let now = t0 + Duration::from_millis(ms);
            spot.observe((x, y), restful(Some(y), taps, now), now);
            taps.update(Some(y), now);
        }
    }

    #[test]
    fn last_spot_keeps_where_the_pointer_rested_before_the_taps() {
        let (mut spot, mut taps, t0) = (LastSpot::default(), TopTaps::default(), Instant::now());
        walk(
            &mut spot,
            &mut taps,
            t0,
            &[
                (0, 700, 600),
                (100, 705, 604),
                (900, 720, 520),
                (910, 720, 300),
                (920, 720, 60),
                (930, 720, 0),
                (980, 720, 162),
                (1400, 720, 100),
                (1410, 720, 0),
            ],
        );
        assert_eq!(spot.get(), Some((705, 604)));
    }

    #[test]
    fn last_spot_ignores_quick_passes_and_pauses_at_the_top() {
        let (mut spot, mut taps, t0) = (LastSpot::default(), TopTaps::default(), Instant::now());
        walk(&mut spot, &mut taps, t0, &[(0, 100, 500), (100, 300, 500)]);
        assert_eq!(spot.get(), None);
        walk(&mut spot, &mut taps, t0, &[(200, 400, 10), (900, 400, 300)]);
        assert_eq!(spot.get(), None);
        let now = t0 + Duration::from_millis(1500);
        spot.observe((-500, 10), restful(None, &taps, now), now);
        assert_eq!(spot.get(), Some((400, 300)));
        let now = t0 + Duration::from_millis(2000);
        spot.observe((-300, 10), restful(None, &taps, now), now);
        assert_eq!(spot.get(), Some((-500, 10)));
    }

    #[test]
    fn a_tap_stops_counting_as_pending_after_the_window() {
        let (mut taps, t0) = (TopTaps::default(), Instant::now());
        taps.update(Some(50), t0);
        taps.update(Some(0), t0);
        assert!(taps.pending(t0 + Duration::from_millis(900)));
        assert!(!taps.pending(t0 + Duration::from_millis(1100)));
    }

    #[test]
    fn arrival_uses_the_last_spot_only_when_it_is_on_the_shown_screen() {
        let lg = Rect {
            x: 0,
            y: 0,
            w: 2560,
            h: 1080,
        };
        assert_eq!(arrival(Some((2000, 700)), lg), (2000, 700));
        assert_eq!(arrival(Some((3000, 700)), lg), (1280, 540));
        assert_eq!(arrival(None, lg), (1280, 540));
    }

    fn desk() -> Vec<Rect> {
        vec![
            Rect {
                x: 0,
                y: 0,
                w: 2560,
                h: 1080,
            },
            Rect {
                x: 2560,
                y: -2160,
                w: 3840,
                h: 2160,
            },
            Rect {
                x: 0,
                y: 1080,
                w: 1920,
                h: 1080,
            },
        ]
    }

    #[test]
    fn distance_is_measured_to_the_open_top_of_the_monitor_under_the_pointer() {
        let d = desk();
        assert_eq!(top_distance(&d, 100, 0), Some(0));
        assert_eq!(top_distance(&d, 100, 500), Some(500));
        assert_eq!(top_distance(&d, 100, -7), Some(-7));
        assert_eq!(top_distance(&d, 3000, -2160), Some(0));
        assert_eq!(top_distance(&d, 3000, -100), Some(2060));
    }

    #[test]
    fn a_top_covered_by_another_monitor_is_not_an_edge() {
        let d = desk();
        assert_eq!(top_distance(&d, 100, 1080), None);
        assert_eq!(top_distance(&d, 1919, 1500), None);
        assert_eq!(top_distance(&d, 9000, 0), None);
    }

    fn tap(t: &mut TopTaps, at: Instant) -> bool {
        t.update(Some(300), at);
        t.update(Some(0), at)
    }

    #[test]
    fn two_taps_within_one_second_fire_once() {
        let start = Instant::now();
        let mut t = TopTaps::default();
        assert!(!tap(&mut t, start));
        assert!(tap(&mut t, start + Duration::from_millis(900)));
        assert!(!tap(&mut t, start + Duration::from_millis(1500)));
    }

    #[test]
    fn a_slow_second_tap_starts_a_new_count() {
        let start = Instant::now();
        let mut t = TopTaps::default();
        assert!(!tap(&mut t, start));
        assert!(!tap(&mut t, start + Duration::from_millis(1100)));
        assert!(tap(&mut t, start + Duration::from_millis(1800)));
    }

    #[test]
    fn staying_or_jittering_at_the_top_is_a_single_tap() {
        let start = Instant::now();
        let mut t = TopTaps::default();
        assert!(!tap(&mut t, start));
        for (i, d) in [0, 3, 0, -5, 7, 0, 1, 0].into_iter().enumerate() {
            let at = start + Duration::from_millis(80 * i as u64);
            assert!(!t.update(Some(d), at), "distância {d}");
        }
        assert!(tap(&mut t, start + Duration::from_millis(900)));
    }

    #[test]
    fn arriving_without_coming_from_below_does_not_count() {
        let start = Instant::now();
        let mut t = TopTaps::default();
        assert!(!t.update(Some(0), start));
        assert!(!tap(&mut t, start));
        t.update(None, start);
        assert!(!t.update(Some(0), start + Duration::from_millis(500)));
        assert!(tap(&mut t, start + Duration::from_millis(600)));
    }
}
