pub const MAX_TEXT: usize = 1 << 20;

pub fn to_wire(text: &str) -> String {
    text.replace("\r\n", "\n")
}

pub fn to_windows(text: &str) -> String {
    to_wire(text).replace('\n', "\r\n")
}

#[derive(Default)]
pub struct ClipSync {
    last: Option<String>,
}

impl ClipSync {
    pub fn outgoing(&mut self, current: Option<&str>) -> Option<String> {
        let text = to_wire(current?);
        if text.is_empty() || text.len() > MAX_TEXT || self.last.as_deref() == Some(text.as_str()) {
            return None;
        }
        self.last = Some(text.clone());
        Some(text)
    }

    pub fn incoming(&mut self, text: &str) {
        self.last = Some(to_wire(text));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_endings_travel_as_lf_and_become_crlf_on_windows() {
        assert_eq!(to_wire("a\r\nb\nc"), "a\nb\nc");
        assert_eq!(to_windows("a\nb\r\nc"), "a\r\nb\r\nc");
        assert_eq!(to_windows("sem quebra"), "sem quebra");
    }

    #[test]
    fn sends_only_when_text_changes() {
        let mut s = ClipSync::default();
        assert_eq!(s.outgoing(Some("olá")), Some("olá".to_string()));
        assert_eq!(s.outgoing(Some("olá")), None);
        assert_eq!(s.outgoing(Some("outro")), Some("outro".to_string()));
    }

    #[test]
    fn received_text_is_not_echoed_back() {
        let mut s = ClipSync::default();
        s.incoming("do outro lado\n");
        assert_eq!(s.outgoing(Some("do outro lado\r\n")), None);
        assert_eq!(s.outgoing(Some("novo")), Some("novo".to_string()));
    }

    #[test]
    fn ignores_missing_empty_and_oversized() {
        let mut s = ClipSync::default();
        assert_eq!(s.outgoing(None), None);
        assert_eq!(s.outgoing(Some("")), None);
        let big = "x".repeat(MAX_TEXT + 1);
        assert_eq!(s.outgoing(Some(&big)), None);
        let limit = "x".repeat(MAX_TEXT);
        assert_eq!(s.outgoing(Some(&limit)), Some(limit.clone()));
    }
}
