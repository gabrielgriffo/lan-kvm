use crate::switch::{Rect, Side};

pub const PERMILLE: i32 = 1000;

pub fn to_permille(offset: i32, len: i32) -> u16 {
    if len <= 1 {
        return 0;
    }
    (offset.clamp(0, len - 1) as i64 * PERMILLE as i64 / (len - 1) as i64) as u16
}

pub fn from_permille(permille: u16, len: i32) -> i32 {
    if len <= 1 {
        return 0;
    }
    (permille.min(PERMILLE as u16) as i64 * (len - 1) as i64 / PERMILLE as i64) as i32
}

pub fn at_facing_edge(x: i32, width: i32, remote_on_right: bool) -> bool {
    if remote_on_right {
        x <= 0
    } else {
        x >= width - 1
    }
}

pub fn arrival_x(width: i32, remote_on_right: bool) -> i32 {
    if remote_on_right {
        1
    } else {
        (width - 2).max(0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PuppetMove {
    Stay,
    Move { x: i32, y: i32 },
    Leave { y: u16 },
}

pub struct Puppet {
    monitors: Vec<Rect>,
    side: Side,
    x: i32,
    y: i32,
}

impl Puppet {
    pub fn enter(monitors: Vec<Rect>, side: Side, preferred: usize, y: u16) -> Option<Self> {
        let probe = |m: &Rect| m.y + m.h / 2;
        let facing = |i: usize| {
            let m = &monitors[i];
            !covered(&monitors, outside_x(m, side), probe(m))
        };
        let index = if preferred < monitors.len() && facing(preferred) {
            preferred
        } else {
            (0..monitors.len()).find(|&i| facing(i))?
        };
        let m = monitors[index];
        let x = match side {
            Side::Right => m.x + m.w - 2,
            Side::Left => m.x + 1,
        };
        let y = m.y + from_permille(y, m.h);
        Some(Self {
            monitors,
            side,
            x,
            y,
        })
    }

    pub fn position(&self) -> (i32, i32) {
        (self.x, self.y)
    }

    pub fn motion(&mut self, dx: i32, dy: i32) -> PuppetMove {
        let steps = dx.abs().max(dy.abs());
        let (sx, sy) = (self.x, self.y);
        let (mut x, mut y) = (sx, sy);
        for i in 1..=steps {
            let (cx, cy) = (sx + dx * i / steps, sy + dy * i / steps);
            if (cx, cy) == (x, y) {
                continue;
            }
            if covered(&self.monitors, cx, cy) {
                (x, y) = (cx, cy);
                continue;
            }
            if cx != x {
                if let Some(m) = self.monitors.iter().copied().find(|m| m.contains(x, y)) {
                    let crossing = match self.side {
                        Side::Right => cx > m.x + m.w - 1,
                        Side::Left => cx < m.x,
                    };
                    if crossing && !covered(&self.monitors, outside_x(&m, self.side), y) {
                        (self.x, self.y) = (x, y);
                        return PuppetMove::Leave {
                            y: to_permille(y - m.y, m.h),
                        };
                    }
                }
            }
            if covered(&self.monitors, cx, y) {
                x = cx;
            } else if covered(&self.monitors, x, cy) {
                y = cy;
            }
        }
        if (x, y) == (self.x, self.y) {
            return PuppetMove::Stay;
        }
        (self.x, self.y) = (x, y);
        PuppetMove::Move { x, y }
    }
}

fn outside_x(m: &Rect, side: Side) -> i32 {
    match side {
        Side::Right => m.x + m.w,
        Side::Left => m.x - 1,
    }
}

fn covered(monitors: &[Rect], x: i32, y: i32) -> bool {
    monitors.iter().any(|m| m.contains(x, y))
}

#[cfg(test)]
mod tests {
    use super::*;

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
                x: -1920,
                y: 0,
                w: 1920,
                h: 1080,
            },
        ]
    }

    #[test]
    fn permille_roundtrip_and_bounds() {
        assert_eq!(to_permille(0, 1080), 0);
        assert_eq!(to_permille(1079, 1080), 1000);
        assert_eq!(to_permille(5000, 1080), 1000);
        assert_eq!(from_permille(1000, 1080), 1079);
        assert_eq!(from_permille(500, 1081), 540);
        assert_eq!(from_permille(9999, 100), 99);
    }

    #[test]
    fn facing_edge_depends_on_where_the_remote_is() {
        assert!(at_facing_edge(0, 4480, true));
        assert!(!at_facing_edge(1, 4480, true));
        assert!(at_facing_edge(4479, 4480, false));
        assert!(!at_facing_edge(0, 4480, false));
        assert_eq!(arrival_x(4480, true), 1);
        assert_eq!(arrival_x(4480, false), 4478);
    }

    #[test]
    fn enters_on_preferred_monitor_edge_at_relative_height() {
        let p = Puppet::enter(desk(), Side::Right, 0, 500).unwrap();
        assert_eq!(p.position(), (2558, 539));
    }

    #[test]
    fn preferred_monitor_without_facing_edge_falls_back() {
        let p = Puppet::enter(desk(), Side::Right, 2, 0).unwrap();
        assert_eq!(p.position(), (2558, 0));
        let p = Puppet::enter(desk(), Side::Left, 0, 1000).unwrap();
        assert_eq!(p.position(), (2561, -1));
    }

    #[test]
    fn moves_across_monitors_and_clamps_at_outer_borders() {
        let mut p = Puppet::enter(desk(), Side::Right, 0, 0).unwrap();
        assert_eq!(p.motion(-100, 10), PuppetMove::Move { x: 2458, y: 10 });
        assert_eq!(p.motion(0, -5000), PuppetMove::Move { x: 2458, y: 0 });
        assert_eq!(p.motion(-3000, 0), PuppetMove::Move { x: -542, y: 0 });
        assert_eq!(p.motion(0, 5000), PuppetMove::Move { x: -542, y: 1079 });
        assert_eq!(p.motion(0, 1), PuppetMove::Stay);
    }

    #[test]
    fn leaves_through_uncovered_facing_edge_only() {
        let mut p = Puppet::enter(desk(), Side::Right, 0, 1000).unwrap();
        assert_eq!(p.motion(-10, -1079), PuppetMove::Move { x: 2548, y: 0 });
        assert_eq!(p.motion(30, 0), PuppetMove::Leave { y: 0 });
        let mut p = Puppet::enter(desk(), Side::Right, 1, 500).unwrap();
        assert_eq!(p.motion(5, 0), PuppetMove::Leave { y: 499 });
    }

    #[test]
    fn internal_border_is_crossed_not_left() {
        let mut p = Puppet::enter(desk(), Side::Left, 2, 0).unwrap();
        assert_eq!(p.motion(2000, 0), PuppetMove::Move { x: 81, y: 0 });
        assert_eq!(p.motion(-2100, 0), PuppetMove::Leave { y: 0 });
    }
}
