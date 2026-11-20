use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Hotkey {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub win: bool,
    pub key: HotkeyKey,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HotkeyKey {
    Character(char),
    Function(u8),
    Space,
}

impl Hotkey {
    pub fn parse(raw: &str) -> Option<Self> {
        let mut ctrl = false;
        let mut shift = false;
        let mut alt = false;
        let mut win = false;
        let mut key = None;

        for token in raw
            .split('+')
            .map(str::trim)
            .filter(|token| !token.is_empty())
        {
            match token.to_ascii_lowercase().as_str() {
                "ctrl" | "control" | "cmd" => {
                    if ctrl {
                        return None;
                    }
                    ctrl = true;
                }
                "shift" => {
                    if shift {
                        return None;
                    }
                    shift = true;
                }
                "alt" | "option" => {
                    if alt {
                        return None;
                    }
                    alt = true;
                }
                "win" | "super" | "meta" => {
                    if win {
                        return None;
                    }
                    win = true;
                }
                _ => {
                    if key.is_some() {
                        return None;
                    }
                    key = parse_key(token);
                }
            }
        }

        if !(ctrl || shift || alt || win) {
            return None;
        }
        Some(Self {
            ctrl,
            shift,
            alt,
            win,
            key: key?,
        })
    }

    pub fn format(&self) -> String {
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("Ctrl".to_string());
        }
        if self.shift {
            parts.push("Shift".to_string());
        }
        if self.alt {
            parts.push("Alt".to_string());
        }
        if self.win {
            parts.push("Win".to_string());
        }
        parts.push(match self.key {
            HotkeyKey::Character(character) => character.to_ascii_uppercase().to_string(),
            HotkeyKey::Function(number) => format!("F{number}"),
            HotkeyKey::Space => "Space".to_string(),
        });
        parts.join("+")
    }

    pub fn vk_code(&self) -> u16 {
        match self.key {
            HotkeyKey::Character(character) => character.to_ascii_uppercase() as u16,
            HotkeyKey::Function(number) => 0x70 + u16::from(number.saturating_sub(1)),
            HotkeyKey::Space => 0x20,
        }
    }
}

impl fmt::Display for Hotkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.format())
    }
}

fn parse_key(raw: &str) -> Option<HotkeyKey> {
    let value = raw.trim();
    if value.eq_ignore_ascii_case("space") {
        return Some(HotkeyKey::Space);
    }
    if value.len() == 1 {
        let character = value.chars().next()?;
        if character.is_ascii_alphanumeric() {
            return Some(HotkeyKey::Character(character.to_ascii_uppercase()));
        }
        return None;
    }
    if let Some(number) = value
        .strip_prefix(['f', 'F'])
        .and_then(|digits| digits.parse::<u8>().ok())
    {
        if (1..=24).contains(&number) {
            return Some(HotkeyKey::Function(number));
        }
    }
    None
}

pub fn format_chord(raw: &str) -> String {
    Hotkey::parse(raw).map_or_else(|| raw.to_string(), |hotkey| hotkey.format())
}

pub fn display_chord(raw: &str) -> String {
    format_chord(raw).replace('+', " + ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_normalizes_defaults() {
        let hotkey = Hotkey::parse("ctrl+shift+d").unwrap();
        assert_eq!(hotkey.format(), "Ctrl+Shift+D");
        assert_eq!(hotkey.vk_code(), u16::from(b'D'));
        assert_eq!(
            Hotkey::parse("Shift + Control + F12").unwrap().format(),
            "Ctrl+Shift+F12"
        );
        assert_eq!(display_chord("Ctrl+Shift+D"), "Ctrl + Shift + D");
    }

    #[test]
    fn rejects_unsafe_or_unsupported_chords() {
        for raw in [
            "D",
            "Shift",
            "Ctrl+Shift",
            "Ctrl+D+D",
            "Ctrl+Shift+é",
            "Ctrl+F25",
            "Ctrl++",
        ] {
            assert!(Hotkey::parse(raw).is_none(), "{raw}");
        }
        assert!(Hotkey::parse("Alt+Space").is_some());
        assert!(Hotkey::parse("Win+1").is_some());
    }

    #[test]
    fn digits_and_function_keys_map_to_win32_codes() {
        assert_eq!(Hotkey::parse("Ctrl+1").unwrap().vk_code(), 0x31);
        assert_eq!(Hotkey::parse("Ctrl+F1").unwrap().vk_code(), 0x70);
        assert_eq!(Hotkey::parse("Ctrl+F24").unwrap().vk_code(), 0x87);
    }
}
