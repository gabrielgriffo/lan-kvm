use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Local,
    Remote,
    Drop,
}

#[derive(Default)]
pub struct KeyRouter {
    local: HashSet<u16>,
    remote: HashSet<u16>,
}

const KEY_BACKSPACE: u16 = 14;
const CTRL: [u16; 2] = [29, 97];
const ALT: [u16; 2] = [56, 100];

impl KeyRouter {
    pub fn is_escape(&self, code: u16, pressed: bool) -> bool {
        let held = |keys: [u16; 2]| keys.iter().any(|k| self.remote.contains(k));
        pressed && code == KEY_BACKSPACE && held(CTRL) && held(ALT)
    }

    pub fn route(&mut self, code: u16, pressed: bool, remote_active: bool) -> Route {
        if pressed {
            if self.local.contains(&code) {
                Route::Local
            } else if self.remote.contains(&code) {
                Route::Drop
            } else if remote_active {
                self.remote.insert(code);
                Route::Remote
            } else {
                self.local.insert(code);
                Route::Local
            }
        } else if self.local.remove(&code) {
            Route::Local
        } else if self.remote.remove(&code) {
            if remote_active {
                Route::Remote
            } else {
                Route::Drop
            }
        } else if remote_active {
            Route::Drop
        } else {
            Route::Local
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Route::*;

    #[test]
    fn follows_active_side() {
        let mut r = KeyRouter::default();
        assert_eq!(r.route(30, true, false), Local);
        assert_eq!(r.route(30, false, false), Local);
        assert_eq!(r.route(30, true, true), Remote);
        assert_eq!(r.route(30, false, true), Remote);
    }

    #[test]
    fn key_held_before_entering_is_released_locally() {
        let mut r = KeyRouter::default();
        assert_eq!(r.route(42, true, false), Local);
        assert_eq!(r.route(42, true, true), Local);
        assert_eq!(r.route(42, false, true), Local);
        assert_eq!(r.route(42, true, true), Remote);
    }

    #[test]
    fn key_held_before_leaving_is_swallowed_locally() {
        let mut r = KeyRouter::default();
        assert_eq!(r.route(29, true, true), Remote);
        assert_eq!(r.route(29, true, false), Drop);
        assert_eq!(r.route(29, false, false), Drop);
        assert_eq!(r.route(29, true, false), Local);
    }

    #[test]
    fn remote_autorepeat_is_dropped() {
        let mut r = KeyRouter::default();
        assert_eq!(r.route(30, true, true), Remote);
        assert_eq!(r.route(30, true, true), Drop);
        assert_eq!(r.route(30, false, true), Remote);
    }

    #[test]
    fn escape_requires_ctrl_and_alt_held_on_remote() {
        let mut r = KeyRouter::default();
        assert!(!r.is_escape(14, true));
        r.route(29, true, true);
        assert!(!r.is_escape(14, true));
        r.route(100, true, true);
        assert!(r.is_escape(14, true));
        assert!(!r.is_escape(14, false));
        assert!(!r.is_escape(15, true));
        r.route(29, false, true);
        assert!(!r.is_escape(14, true));
    }

    #[test]
    fn escape_ignores_modifiers_held_locally() {
        let mut r = KeyRouter::default();
        r.route(97, true, false);
        r.route(56, true, false);
        assert!(!r.is_escape(14, true));
    }

    #[test]
    fn stray_release_passes_only_when_local() {
        let mut r = KeyRouter::default();
        assert_eq!(r.route(50, false, false), Local);
        assert_eq!(r.route(50, false, true), Drop);
    }
}
