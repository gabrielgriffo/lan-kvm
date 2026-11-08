use crate::switch::Side;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub address: String,
    pub side: Side,
    pub edges: bool,
    pub enabled: bool,
    pub protect_corners: bool,
    pub monitor_switch: bool,
    pub monitor_guard: bool,
    pub monitor: String,
    pub pc_input: u8,
    pub notebook_input: u8,
    pub tap_switch: bool,
    pub tap_monitor: String,
    pub tap_pc_input: u8,
    pub tap_notebook_input: u8,
    pub audio: bool,
}

fn parse_byte(value: &str) -> Option<u8> {
    match value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        Some(hex) => u8::from_str_radix(hex, 16).ok(),
        None => value.parse().ok(),
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            address: String::new(),
            side: Side::Right,
            edges: true,
            enabled: true,
            protect_corners: false,
            monitor_switch: false,
            monitor_guard: true,
            monitor: String::new(),
            pc_input: 0,
            notebook_input: 0,
            tap_switch: false,
            tap_monitor: String::new(),
            tap_pc_input: 0,
            tap_notebook_input: 0,
            audio: false,
        }
    }
}

impl Config {
    pub fn parse(text: &str) -> Self {
        let mut config = Self::default();
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "address" => config.address = value.to_string(),
                "side" => match value {
                    "left" => config.side = Side::Left,
                    "right" => config.side = Side::Right,
                    _ => {}
                },
                "edges" => match value {
                    "true" => config.edges = true,
                    "false" => config.edges = false,
                    _ => {}
                },
                "enabled" => match value {
                    "true" => config.enabled = true,
                    "false" => config.enabled = false,
                    _ => {}
                },
                "protect_corners" => match value {
                    "true" => config.protect_corners = true,
                    "false" => config.protect_corners = false,
                    _ => {}
                },
                "monitor_switch" => match value {
                    "true" => config.monitor_switch = true,
                    "false" => config.monitor_switch = false,
                    _ => {}
                },
                "monitor_guard" => match value {
                    "true" => config.monitor_guard = true,
                    "false" => config.monitor_guard = false,
                    _ => {}
                },
                "monitor" => config.monitor = value.to_string(),
                "pc_input" => {
                    if let Some(v) = parse_byte(value) {
                        config.pc_input = v;
                    }
                }
                "notebook_input" => {
                    if let Some(v) = parse_byte(value) {
                        config.notebook_input = v;
                    }
                }
                "tap_switch" => match value {
                    "true" => config.tap_switch = true,
                    "false" => config.tap_switch = false,
                    _ => {}
                },
                "tap_monitor" => config.tap_monitor = value.to_string(),
                "audio" => match value {
                    "true" => config.audio = true,
                    "false" => config.audio = false,
                    _ => {}
                },
                "tap_pc_input" => {
                    if let Some(v) = parse_byte(value) {
                        config.tap_pc_input = v;
                    }
                }
                "tap_notebook_input" => {
                    if let Some(v) = parse_byte(value) {
                        config.tap_notebook_input = v;
                    }
                }
                _ => {}
            }
        }
        config
    }

    pub fn serialize(&self) -> String {
        let side = match self.side {
            Side::Left => "left",
            Side::Right => "right",
        };
        format!(
            "address = {}\nside = {side}\nedges = {}\nenabled = {}\nprotect_corners = {}\n\
             monitor_switch = {}\nmonitor_guard = {}\nmonitor = {}\npc_input = 0x{:02X}\nnotebook_input = 0x{:02X}\n\
             tap_switch = {}\ntap_monitor = {}\ntap_pc_input = 0x{:02X}\ntap_notebook_input = 0x{:02X}\naudio = {}\n",
            self.address.trim(),
            self.edges,
            self.enabled,
            self.protect_corners,
            self.monitor_switch,
            self.monitor_guard,
            self.monitor.trim(),
            self.pc_input,
            self.notebook_input,
            self.tap_switch,
            self.tap_monitor.trim(),
            self.tap_pc_input,
            self.tap_notebook_input,
            self.audio
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let config = Config {
            address: "192.168.0.18:24801".to_string(),
            side: Side::Left,
            edges: false,
            enabled: false,
            protect_corners: true,
            monitor_switch: true,
            monitor_guard: false,
            monitor: "LG 34GL750".to_string(),
            pc_input: 0x0F,
            notebook_input: 0x12,
            tap_switch: true,
            tap_monitor: "DELL U2723QE".to_string(),
            tap_pc_input: 0x0F,
            tap_notebook_input: 0x11,
            audio: true,
        };
        assert_eq!(Config::parse(&config.serialize()), config);
    }

    #[test]
    fn empty_text_gives_defaults() {
        assert_eq!(Config::parse(""), Config::default());
    }

    #[test]
    fn tolerates_spacing_comments_and_unknown_keys() {
        let text = "# comentário\n  address=notebook.local \n\ncor = azul\nside =left\nlixo\n";
        let config = Config::parse(text);
        assert_eq!(config.address, "notebook.local");
        assert_eq!(config.side, Side::Left);
        assert!(config.enabled);
    }

    #[test]
    fn inputs_accept_hex_or_decimal_and_ignore_garbage() {
        let config = Config::parse("pc_input = 0x0f\nnotebook_input = 18\n");
        assert_eq!((config.pc_input, config.notebook_input), (0x0F, 0x12));
        let config = Config::parse("pc_input = 0xZZ\nnotebook_input = 999\nmonitor_switch = sim\n");
        assert_eq!((config.pc_input, config.notebook_input), (0, 0));
        assert!(!config.monitor_switch);
        assert!(config.monitor_guard);
        assert!(Config::default().serialize().contains("pc_input = 0x00"));
        assert!(!Config::parse("monitor_guard = false\n").monitor_guard);
    }

    #[test]
    fn invalid_values_keep_defaults() {
        let config = Config::parse("side = cima\nenabled = talvez\nprotect_corners = sim\n");
        assert_eq!(config.side, Side::Right);
        assert!(config.enabled);
        assert!(!config.protect_corners);
        assert!(config.edges);
        assert!(!Config::parse("edges = false\n").edges);
        assert!(Config::parse("edges = nao\n").edges);
    }
}
