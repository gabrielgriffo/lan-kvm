#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }

    pub fn center(&self) -> (i32, i32) {
        (self.x + self.w / 2, self.y + self.h / 2)
    }

    fn right(&self) -> i32 {
        self.x + self.w - 1
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Pass,
    Block,
    Enter { x: u16, y: u16 },
    Motion { x: u16, y: u16 },
    Leave { x: i32, y: i32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Local,
    Remote { x: i32, y: i32, entry: usize },
}

pub struct Switcher {
    monitors: Vec<Rect>,
    remote_w: i32,
    remote_h: i32,
    side: Side,
    state: State,
    corner_guard: bool,
    edges: bool,
}

pub const CORNER_PERCENT: i32 = 5;

pub fn in_corner(py: i32, m: &Rect) -> bool {
    let band = m.h * CORNER_PERCENT / 100;
    py < m.y + band || py >= m.y + m.h - band
}

fn scale(v: i32, from_len: i32, to_len: i32) -> i32 {
    if from_len <= 1 || to_len <= 1 {
        return 0;
    }
    (v.clamp(0, from_len - 1) as i64 * (to_len - 1) as i64 / (from_len - 1) as i64) as i32
}

impl Switcher {
    pub fn new(monitors: Vec<Rect>, remote: (u16, u16), side: Side) -> Self {
        Self {
            monitors,
            remote_w: remote.0.max(1) as i32,
            remote_h: remote.1.max(1) as i32,
            side,
            state: State::Local,
            corner_guard: false,
            edges: true,
        }
    }

    pub fn set_corner_guard(&mut self, on: bool) {
        self.corner_guard = on;
    }

    pub fn set_edges(&mut self, on: bool) {
        self.edges = on;
    }

    pub fn edges(&self) -> bool {
        self.edges
    }

    pub fn is_remote(&self) -> bool {
        matches!(self.state, State::Remote { .. })
    }

    pub fn remote_position(&self) -> Option<(i32, i32)> {
        match self.state {
            State::Remote { x, y, .. } => Some((x, y)),
            State::Local => None,
        }
    }

    pub fn remote_area(&self) -> Rect {
        Rect {
            x: 0,
            y: 0,
            w: self.remote_w,
            h: self.remote_h,
        }
    }

    pub fn force_enter(&mut self, entry: usize) -> Option<Action> {
        self.force_enter_at(entry, self.remote_area().center())
    }

    pub fn force_enter_at(&mut self, entry: usize, (x, y): (i32, i32)) -> Option<Action> {
        if self.is_remote() || entry >= self.monitors.len() {
            return None;
        }
        let x = x.clamp(0, self.remote_w - 1);
        let y = y.clamp(0, self.remote_h - 1);
        self.state = State::Remote { x, y, entry };
        Some(Action::Enter {
            x: x as u16,
            y: y as u16,
        })
    }

    pub fn force_leave(&mut self) -> Option<Action> {
        match self.state {
            State::Remote { y, entry, .. } => Some(self.leave(entry, y)),
            State::Local => None,
        }
    }

    pub fn on_motion(&mut self, px: i32, py: i32) -> Action {
        match self.state {
            State::Local => match self.edge_monitor(px, py) {
                Some(i) => self.enter(i, py),
                None => Action::Pass,
            },
            State::Remote { .. } => Action::Block,
        }
    }

    pub fn on_delta(&mut self, dx: i32, dy: i32) -> Action {
        match self.state {
            State::Local => Action::Pass,
            State::Remote { x, y, entry } => {
                let nx = x + dx;
                let ny = (y + dy).clamp(0, self.remote_h - 1);
                let leaving = match self.side {
                    Side::Right => nx < 0,
                    Side::Left => nx >= self.remote_w,
                };
                if leaving && self.edges {
                    return self.leave(entry, ny);
                }
                let nx = nx.clamp(0, self.remote_w - 1);
                if (nx, ny) == (x, y) {
                    return Action::Block;
                }
                self.state = State::Remote {
                    x: nx,
                    y: ny,
                    entry,
                };
                Action::Motion {
                    x: nx as u16,
                    y: ny as u16,
                }
            }
        }
    }

    fn covered(&self, px: i32, py: i32) -> bool {
        self.monitors.iter().any(|m| m.contains(px, py))
    }

    fn edge_monitor(&self, px: i32, py: i32) -> Option<usize> {
        if !self.edges {
            return None;
        }
        self.monitors.iter().position(|m| {
            if py < m.y || py >= m.y + m.h || (self.corner_guard && in_corner(py, m)) {
                return false;
            }
            let (edge, outside) = match self.side {
                Side::Right => (px >= m.right(), m.right() + 1),
                Side::Left => (px <= m.x, m.x - 1),
            };
            edge && !self.covered(outside, py) && (m.contains(px, py) || !self.covered(px, py))
        })
    }

    fn enter(&mut self, entry: usize, py: i32) -> Action {
        let m = self.monitors[entry];
        let x = match self.side {
            Side::Right => 0,
            Side::Left => self.remote_w - 1,
        };
        let y = scale(py - m.y, m.h, self.remote_h);
        self.state = State::Remote { x, y, entry };
        Action::Enter {
            x: x as u16,
            y: y as u16,
        }
    }

    fn leave(&mut self, entry: usize, ry: i32) -> Action {
        let m = self.monitors[entry];
        let x = match self.side {
            Side::Right => m.right() - 1,
            Side::Left => m.x + 1,
        };
        let y = m.y + scale(ry, self.remote_h, m.h);
        self.state = State::Local;
        Action::Leave { x, y }
    }
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

    fn right() -> Switcher {
        Switcher::new(desk(), (1920, 1200), Side::Right)
    }

    #[test]
    fn stays_local_away_from_edges() {
        let mut s = right();
        assert_eq!(s.on_motion(100, 100), Action::Pass);
        assert_eq!(s.on_motion(2558, 1000), Action::Pass);
        assert!(!s.is_remote());
    }

    #[test]
    fn internal_border_between_monitors_does_not_switch() {
        let mut s = right();
        assert_eq!(s.on_motion(-1, 500), Action::Pass);
        assert_eq!(s.on_motion(2559, -10), Action::Pass);
    }

    #[test]
    fn left_edge_does_not_switch_when_remote_is_on_the_right() {
        let mut s = right();
        assert_eq!(s.on_motion(-1920, 500), Action::Pass);
    }

    #[test]
    fn enters_from_right_edge_of_primary_scaling_y() {
        let mut s = right();
        assert_eq!(s.on_motion(2559, 1079), Action::Enter { x: 0, y: 1199 });
        assert!(s.is_remote());
    }

    #[test]
    fn enters_from_right_edge_of_upper_monitor() {
        let mut s = right();
        assert_eq!(s.on_motion(6399, -1080), Action::Enter { x: 0, y: 599 });
    }

    #[test]
    fn overshoot_past_edge_still_enters() {
        let mut s = right();
        assert!(matches!(s.on_motion(2565, 0), Action::Enter { .. }));
    }

    #[test]
    fn deltas_move_the_remote_pointer_and_clamp() {
        let mut s = right();
        s.on_motion(2559, 0);
        assert_eq!(s.on_delta(30, 10), Action::Motion { x: 30, y: 10 });
        assert_eq!(s.remote_position(), Some((30, 10)));
        assert_eq!(s.on_delta(5000, -50), Action::Motion { x: 1919, y: 0 });
        assert_eq!(s.on_delta(1, -1), Action::Block);
        assert_eq!(s.on_delta(0, 5000), Action::Motion { x: 1919, y: 1199 });
    }

    #[test]
    fn leaves_back_to_entry_monitor() {
        let mut s = right();
        s.on_motion(2559, 0);
        s.on_delta(10, 600);
        assert_eq!(s.on_delta(-20, 0), Action::Leave { x: 2558, y: 539 });
        assert!(!s.is_remote());
    }

    #[test]
    fn pointer_position_is_ignored_while_remote_and_deltas_while_local() {
        let mut s = right();
        assert_eq!(s.on_delta(50, 0), Action::Pass);
        s.on_motion(2559, 0);
        assert_eq!(s.on_motion(100, 100), Action::Block);
        assert_eq!(s.on_motion(2559, 0), Action::Block);
        assert!(s.is_remote());
    }

    #[test]
    fn force_leave_returns_to_entry_edge_only_when_remote() {
        let mut s = right();
        assert_eq!(s.force_leave(), None);
        assert_eq!(s.remote_position(), None);
        s.on_motion(6399, -2160);
        s.on_delta(100, 1199);
        assert_eq!(s.force_leave(), Some(Action::Leave { x: 6398, y: -1 }));
        assert!(!s.is_remote());
        assert_eq!(s.force_leave(), None);
    }

    #[test]
    fn force_enter_puts_the_pointer_in_the_middle_and_leaves_through_the_entry_monitor() {
        let mut s = right();
        assert_eq!(s.force_enter(9), None);
        assert_eq!(s.force_enter(1), Some(Action::Enter { x: 960, y: 600 }));
        assert!(s.is_remote());
        assert_eq!(s.force_enter(0), None);
        assert_eq!(s.remote_position(), Some((960, 600)));
        assert_eq!(s.on_delta(10, 0), Action::Motion { x: 970, y: 600 });
        let Some(Action::Leave { x, y }) = s.force_leave() else {
            panic!("deveria sair");
        };
        assert!(desk()[1].contains(x, y));
        assert_eq!(desk()[1].center(), (4480, -1080));
    }

    #[test]
    fn corner_guard_blocks_top_and_bottom_five_percent_of_each_monitor() {
        let mut s = right();
        s.set_corner_guard(true);
        assert_eq!(s.on_motion(2559, 0), Action::Pass);
        assert_eq!(s.on_motion(2559, 53), Action::Pass);
        assert_eq!(s.on_motion(2559, 1026), Action::Pass);
        assert_eq!(s.on_motion(2559, 1079), Action::Pass);
        assert_eq!(s.on_motion(6399, -2160), Action::Pass);
        assert_eq!(s.on_motion(6399, -2053), Action::Pass);
        assert_eq!(s.on_motion(6399, -108), Action::Pass);
        assert!(!s.is_remote());
        assert!(matches!(s.on_motion(6399, -2052), Action::Enter { .. }));
        s.force_leave();
        assert!(matches!(s.on_motion(2559, 54), Action::Enter { .. }));
        s.force_leave();
        assert!(matches!(s.on_motion(2559, 1025), Action::Enter { .. }));
    }

    #[test]
    fn corner_guard_off_switches_everywhere_and_can_be_toggled() {
        let mut s = right();
        assert!(matches!(s.on_motion(2559, 0), Action::Enter { .. }));
        s.force_leave();
        s.set_corner_guard(true);
        assert_eq!(s.on_motion(2559, 0), Action::Pass);
        s.set_corner_guard(false);
        assert!(matches!(s.on_motion(2559, 1079), Action::Enter { .. }));
    }

    #[test]
    fn force_enter_at_places_the_pointer_inside_the_remote_screen() {
        let mut s = right();
        assert_eq!(
            s.force_enter_at(0, (300, 400)),
            Some(Action::Enter { x: 300, y: 400 })
        );
        assert_eq!(s.remote_position(), Some((300, 400)));
        assert_eq!(s.force_enter_at(0, (10, 10)), None);
        s.force_leave();
        assert_eq!(
            s.force_enter_at(0, (5000, -20)),
            Some(Action::Enter { x: 1919, y: 0 })
        );
    }

    #[test]
    fn without_edges_only_forced_moves_switch() {
        let mut s = right();
        s.set_edges(false);
        assert_eq!(s.on_motion(2559, 500), Action::Pass);
        assert_eq!(s.on_motion(2600, 500), Action::Pass);
        assert!(matches!(s.force_enter(0), Some(Action::Enter { .. })));
        assert_eq!(s.on_delta(-5000, 0), Action::Motion { x: 0, y: 600 });
        assert_eq!(s.on_delta(-10, 0), Action::Block);
        assert!(s.is_remote());
        assert!(matches!(s.force_leave(), Some(Action::Leave { .. })));
        assert!(!s.is_remote());
        s.set_edges(true);
        assert!(matches!(s.on_motion(2559, 500), Action::Enter { .. }));
    }

    #[test]
    fn corner_guard_mirrors_on_the_left_side() {
        let mut s = Switcher::new(desk(), (1000, 1000), Side::Left);
        s.set_corner_guard(true);
        assert_eq!(s.on_motion(-1920, 10), Action::Pass);
        assert_eq!(s.on_motion(-1920, 1070), Action::Pass);
        assert!(matches!(s.on_motion(-1920, 540), Action::Enter { .. }));
    }

    #[test]
    fn left_side_mirrors_behaviour() {
        let mut s = Switcher::new(desk(), (1000, 1000), Side::Left);
        assert_eq!(s.on_motion(0, 500), Action::Pass);
        assert_eq!(s.on_motion(-1920, 0), Action::Enter { x: 999, y: 0 });
        assert_eq!(s.on_delta(-9, 0), Action::Motion { x: 990, y: 0 });
        assert_eq!(s.on_delta(10, 0), Action::Leave { x: -1919, y: 0 });
    }
}
