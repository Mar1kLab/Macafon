//! Shortcut strings (HK-002): `Ctrl+Alt+M`, parsed into modifiers and one key, printed in a
//! canonical order, and translated for the portal (xdg shortcuts spec) and X11.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Modifiers {
    pub ctrl: bool,
    pub ctrl_side: u8,
    pub alt_side: u8,
    pub alt: bool,
    pub shift: bool,
    pub super_: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Accel {
    pub modifiers: Modifiers,
    /// Canonical key: `A`…`Z`, `0`…`9`, `F1`…`F24`, `Space`, `PageUp`, `ArrowUp`, `Backquote`, …
    pub key: String,
}

/// Named keys accepted besides letters, digits and F-keys.
const NAMED: [&str; 15] = [
    "Space",
    "Insert",
    "Delete",
    "Home",
    "End",
    "PageUp",
    "PageDown",
    "ArrowUp",
    "ArrowDown",
    "ArrowLeft",
    "ArrowRight",
    "Pause",
    "ScrollLock",
    "PrintScreen",
    // The key left of 1 (`` ` `` on US layouts, `ё` on Russian ones).
    "Backquote",
];

/// Keys that type a character: alone they would steal typing.
fn types(key: &str) -> bool {
    key.len() == 1 || key == "Backquote"
}

fn function_key(key: &str) -> Option<u8> {
    let n: u8 = key.strip_prefix('F')?.parse().ok()?;
    (1..=24).contains(&n).then_some(n)
}

fn canonical_key(raw: &str) -> Option<String> {
    let upper = raw.to_ascii_uppercase();
    if raw.len() == 1 && raw.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Some(upper);
    }
    if function_key(&upper).is_some() {
        return Some(upper);
    }
    let alias = match upper.as_str() {
        "PGUP" | "PAGE_UP" => "PageUp",
        "PGDN" | "PAGE_DOWN" => "PageDown",
        "UP" => "ArrowUp",
        "DOWN" => "ArrowDown",
        "LEFT" => "ArrowLeft",
        "RIGHT" => "ArrowRight",
        "DEL" => "Delete",
        "INS" => "Insert",
        "PRINT" => "PrintScreen",
        "`" | "GRAVE" => "Backquote",
        _ => "",
    };
    if !alias.is_empty() {
        return Some(alias.to_owned());
    }
    NAMED
        .iter()
        .find(|name| name.eq_ignore_ascii_case(raw))
        .map(|name| (*name).to_owned())
}

impl Accel {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut modifiers = Modifiers::default();
        let mut key = None;
        for part in text.split('+').map(str::trim) {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => modifiers.ctrl = true,
                "lctrl" | "leftctrl" => {
                    modifiers.ctrl = true;
                    modifiers.ctrl_side |= 1;
                }
                "rctrl" | "rightctrl" => {
                    modifiers.ctrl = true;
                    modifiers.ctrl_side |= 2;
                }
                "lalt" | "leftalt" => {
                    modifiers.alt = true;
                    modifiers.alt_side |= 1;
                }
                "ralt" | "rightalt" => {
                    modifiers.alt = true;
                    modifiers.alt_side |= 2;
                }
                "alt" => modifiers.alt = true,
                "shift" => modifiers.shift = true,
                "super" | "meta" | "logo" | "win" => modifiers.super_ = true,
                "" => return Err(format!("empty key in {text:?}")),
                _ if key.is_some() => return Err(format!("{text:?} has more than one key")),
                _ => {
                    key = Some(canonical_key(part).ok_or_else(|| format!("unknown key {part:?}"))?)
                }
            }
        }
        let key = key.ok_or_else(|| format!("{text:?} has no key"))?;
        let any_modifier = modifiers.ctrl || modifiers.alt || modifiers.super_;
        // A bare letter or digit would steal typing; F-keys and navigation keys alone are fine.
        if !any_modifier && types(&key) {
            return Err(format!("{text:?} needs Ctrl, Alt or Super"));
        }
        Ok(Self { modifiers, key })
    }

    pub fn sided(&self) -> bool {
        self.modifiers.ctrl_side != 0 || self.modifiers.alt_side != 0
    }

    pub fn overlaps(&self, other: &Self) -> bool {
        let (a, b) = (self.modifiers, other.modifiers);
        self.key == other.key
            && a.ctrl == b.ctrl
            && a.alt == b.alt
            && a.shift == b.shift
            && a.super_ == b.super_
            && (a.ctrl_side == 0 || b.ctrl_side == 0 || a.ctrl_side == b.ctrl_side)
            && (a.alt_side == 0 || b.alt_side == 0 || a.alt_side == b.alt_side)
    }

    /// xdg shortcuts spec trigger: `CTRL+ALT+m`, keys as XKB keysym names.
    #[cfg(any(target_os = "linux", test))]
    pub fn portal_trigger(&self) -> String {
        let mut parts = Vec::new();
        let m = self.modifiers;
        if m.ctrl {
            parts.push("CTRL".to_owned());
        }
        if m.alt {
            parts.push("ALT".to_owned());
        }
        if m.shift {
            parts.push("SHIFT".to_owned());
        }
        if m.super_ {
            parts.push("LOGO".to_owned());
        }
        let keysym = match self.key.as_str() {
            key if key.len() == 1 => key.to_ascii_lowercase(),
            "Space" => "space".into(),
            "PageUp" => "Prior".into(),
            "PageDown" => "Next".into(),
            "ArrowUp" => "Up".into(),
            "ArrowDown" => "Down".into(),
            "ArrowLeft" => "Left".into(),
            "ArrowRight" => "Right".into(),
            "ScrollLock" => "Scroll_Lock".into(),
            "PrintScreen" => "Print".into(),
            "Backquote" => "grave".into(),
            other => other.to_owned(),
        };
        parts.push(keysym);
        parts.join("+")
    }
}

impl std::fmt::Display for Accel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let m = self.modifiers;
        for (on, name) in [
            (m.ctrl && m.ctrl_side == 0, "Ctrl"),
            (m.ctrl_side & 1 != 0, "LCtrl"),
            (m.ctrl_side & 2 != 0, "RCtrl"),
            (m.alt && m.alt_side == 0, "Alt"),
            (m.alt_side & 1 != 0, "LAlt"),
            (m.alt_side & 2 != 0, "RAlt"),
            (m.shift, "Shift"),
            (m.super_, "Super"),
        ] {
            if on {
                write!(f, "{name}+")?;
            }
        }
        f.write_str(&self.key)
    }
}

pub fn hotkey(accel: &Accel) -> Result<global_hotkey::hotkey::HotKey, String> {
    let m = accel.modifiers;
    let mut text = String::new();
    for (on, name) in [
        (m.ctrl, "control"),
        (m.alt, "alt"),
        (m.shift, "shift"),
        (m.super_, "super"),
    ] {
        if on {
            text.push_str(name);
            text.push('+');
        }
    }
    let key = &accel.key;
    let code = if key.len() == 1 && key.chars().all(|c| c.is_ascii_digit()) {
        format!("Digit{key}")
    } else if key.len() == 1 {
        format!("Key{key}")
    } else {
        key.clone()
    };
    text.push_str(&code);
    <global_hotkey::hotkey::HotKey as std::str::FromStr>::from_str(&text)
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sided_modifiers_remain_distinct_and_generic_bindings_conflict() {
        let left = Accel::parse("LCtrl+LAlt+M").expect("left");
        let right = Accel::parse("RCtrl+RAlt+M").expect("right");
        assert_eq!(left.to_string(), "LCtrl+LAlt+M");
        assert_eq!(right.to_string(), "RCtrl+RAlt+M");
        assert!(!left.overlaps(&right));
        assert!(left.overlaps(&Accel::parse("Ctrl+Alt+M").expect("either")));
    }

    #[test]
    fn native_mapping_preserves_modifiers_and_key_codes() {
        use global_hotkey::hotkey::{Code, HotKey, Modifiers};
        for (text, key, mods) in [
            ("Ctrl+Alt+F9", Code::F9, Modifiers::CONTROL | Modifiers::ALT),
            ("Ctrl+M", Code::KeyM, Modifiers::CONTROL),
            (
                "Super+Shift+1",
                Code::Digit1,
                Modifiers::SUPER | Modifiers::SHIFT,
            ),
        ] {
            assert_eq!(
                hotkey(&Accel::parse(text).expect("valid accelerator")),
                Ok(HotKey::new(Some(mods), key))
            );
        }
        for key in NAMED {
            let accel = Accel::parse(&format!("Ctrl+Alt+{key}")).expect("valid named key");
            assert!(hotkey(&accel).is_ok(), "{key}");
        }
        assert_eq!(
            hotkey(&Accel::parse("Ctrl+Alt+`").expect("console key")),
            Ok(HotKey::new(
                Some(Modifiers::CONTROL | Modifiers::ALT),
                Code::Backquote
            ))
        );
    }

    #[test]
    fn shortcuts_parse_to_one_canonical_form() {
        let accel = Accel::parse("alt + ctrl+m").expect("valid");
        assert_eq!(accel.to_string(), "Ctrl+Alt+M");
        assert_eq!(accel.portal_trigger(), "CTRL+ALT+m");
        assert_eq!(
            Accel::parse("Super+Shift+pgup").expect("valid").to_string(),
            "Shift+Super+PageUp"
        );
        assert_eq!(
            Accel::parse("F9").expect("F-key alone").portal_trigger(),
            "F9"
        );
        assert!(Accel::parse("M").is_err());
        assert!(Accel::parse("Backquote").is_err());
        assert_eq!(
            Accel::parse("ctrl+alt+backquote")
                .expect("valid")
                .portal_trigger(),
            "CTRL+ALT+grave"
        );
        assert!(Accel::parse("Shift+M").is_err());
        assert!(Accel::parse("Ctrl+M+N").is_err());
        assert!(Accel::parse("Ctrl+Banana").is_err());
    }
}
