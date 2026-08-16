#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyDefinition {
    pub code: &'static str,
    pub key: &'static str,
    pub key_code: u32,
    pub key_code_without_location: Option<u32>,
    pub text: Option<&'static str>,
    pub location: u32,
    pub shifted: Option<ShiftedKeyDefinition>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShiftedKeyDefinition {
    pub key: &'static str,
    pub key_code: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyDescription {
    pub code: &'static str,
    pub key: &'static str,
    pub key_code: u32,
    pub key_code_without_location: u32,
    pub text: &'static str,
    pub location: u32,
}

pub const KEYPAD_LOCATION: u32 = 3;

macro_rules! key {
    ($code:literal, $key:literal, $key_code:literal) => {
        KeyDefinition {
            code: $code,
            key: $key,
            key_code: $key_code,
            key_code_without_location: None,
            text: None,
            location: 0,
            shifted: None,
        }
    };
    ($code:literal, $key:literal, $key_code:literal, shifted = $shifted:literal) => {
        KeyDefinition {
            code: $code,
            key: $key,
            key_code: $key_code,
            key_code_without_location: None,
            text: None,
            location: 0,
            shifted: Some(ShiftedKeyDefinition {
                key: $shifted,
                key_code: None,
            }),
        }
    };
    ($code:literal, $key:literal, $key_code:literal, text = $text:literal) => {
        KeyDefinition {
            code: $code,
            key: $key,
            key_code: $key_code,
            key_code_without_location: None,
            text: Some($text),
            location: 0,
            shifted: None,
        }
    };
    ($code:literal, $key:literal, $key_code:literal, location = $location:literal) => {
        KeyDefinition {
            code: $code,
            key: $key,
            key_code: $key_code,
            key_code_without_location: None,
            text: None,
            location: $location,
            shifted: None,
        }
    };
    ($code:literal, $key:literal, $key_code:literal, without_location = $without:literal, location = $location:literal) => {
        KeyDefinition {
            code: $code,
            key: $key,
            key_code: $key_code,
            key_code_without_location: Some($without),
            text: None,
            location: $location,
            shifted: None,
        }
    };
    ($code:literal, $key:literal, $key_code:literal, text = $text:literal, location = $location:literal) => {
        KeyDefinition {
            code: $code,
            key: $key,
            key_code: $key_code,
            key_code_without_location: None,
            text: Some($text),
            location: $location,
            shifted: None,
        }
    };
    ($code:literal, $key:literal, $key_code:literal, shifted = $shifted:literal, shifted_code = $shifted_code:literal, location = $location:literal) => {
        KeyDefinition {
            code: $code,
            key: $key,
            key_code: $key_code,
            key_code_without_location: None,
            text: None,
            location: $location,
            shifted: Some(ShiftedKeyDefinition {
                key: $shifted,
                key_code: Some($shifted_code),
            }),
        }
    };
}

pub static US_KEYBOARD_LAYOUT: &[KeyDefinition] = &[
    key!("Escape", "Escape", 27),
    key!("F1", "F1", 112),
    key!("F2", "F2", 113),
    key!("F3", "F3", 114),
    key!("F4", "F4", 115),
    key!("F5", "F5", 116),
    key!("F6", "F6", 117),
    key!("F7", "F7", 118),
    key!("F8", "F8", 119),
    key!("F9", "F9", 120),
    key!("F10", "F10", 121),
    key!("F11", "F11", 122),
    key!("F12", "F12", 123),
    key!("Backquote", "`", 192, shifted = "~"),
    key!("Digit1", "1", 49, shifted = "!"),
    key!("Digit2", "2", 50, shifted = "@"),
    key!("Digit3", "3", 51, shifted = "#"),
    key!("Digit4", "4", 52, shifted = "$"),
    key!("Digit5", "5", 53, shifted = "%"),
    key!("Digit6", "6", 54, shifted = "^"),
    key!("Digit7", "7", 55, shifted = "&"),
    key!("Digit8", "8", 56, shifted = "*"),
    key!("Digit9", "9", 57, shifted = "("),
    key!("Digit0", "0", 48, shifted = ")"),
    key!("Minus", "-", 189, shifted = "_"),
    key!("Equal", "=", 187, shifted = "+"),
    key!("Backslash", "\\", 220, shifted = "|"),
    key!("Backspace", "Backspace", 8),
    key!("Tab", "Tab", 9),
    key!("KeyQ", "q", 81, shifted = "Q"),
    key!("KeyW", "w", 87, shifted = "W"),
    key!("KeyE", "e", 69, shifted = "E"),
    key!("KeyR", "r", 82, shifted = "R"),
    key!("KeyT", "t", 84, shifted = "T"),
    key!("KeyY", "y", 89, shifted = "Y"),
    key!("KeyU", "u", 85, shifted = "U"),
    key!("KeyI", "i", 73, shifted = "I"),
    key!("KeyO", "o", 79, shifted = "O"),
    key!("KeyP", "p", 80, shifted = "P"),
    key!("BracketLeft", "[", 219, shifted = "{"),
    key!("BracketRight", "]", 221, shifted = "}"),
    key!("CapsLock", "CapsLock", 20),
    key!("KeyA", "a", 65, shifted = "A"),
    key!("KeyS", "s", 83, shifted = "S"),
    key!("KeyD", "d", 68, shifted = "D"),
    key!("KeyF", "f", 70, shifted = "F"),
    key!("KeyG", "g", 71, shifted = "G"),
    key!("KeyH", "h", 72, shifted = "H"),
    key!("KeyJ", "j", 74, shifted = "J"),
    key!("KeyK", "k", 75, shifted = "K"),
    key!("KeyL", "l", 76, shifted = "L"),
    key!("Semicolon", ";", 186, shifted = ":"),
    key!("Quote", "'", 222, shifted = "\""),
    key!("Enter", "Enter", 13, text = "\r"),
    key!(
        "ShiftLeft",
        "Shift",
        160,
        without_location = 16,
        location = 1
    ),
    key!("KeyZ", "z", 90, shifted = "Z"),
    key!("KeyX", "x", 88, shifted = "X"),
    key!("KeyC", "c", 67, shifted = "C"),
    key!("KeyV", "v", 86, shifted = "V"),
    key!("KeyB", "b", 66, shifted = "B"),
    key!("KeyN", "n", 78, shifted = "N"),
    key!("KeyM", "m", 77, shifted = "M"),
    key!("Comma", ",", 188, shifted = "<"),
    key!("Period", ".", 190, shifted = ">"),
    key!("Slash", "/", 191, shifted = "?"),
    key!(
        "ShiftRight",
        "Shift",
        161,
        without_location = 16,
        location = 2
    ),
    key!(
        "ControlLeft",
        "Control",
        162,
        without_location = 17,
        location = 1
    ),
    key!("MetaLeft", "Meta", 91, location = 1),
    key!("AltLeft", "Alt", 164, without_location = 18, location = 1),
    key!("Space", " ", 32),
    key!("AltRight", "Alt", 165, without_location = 18, location = 2),
    key!("AltGraph", "AltGraph", 225),
    key!("MetaRight", "Meta", 92, location = 2),
    key!("ContextMenu", "ContextMenu", 93),
    key!(
        "ControlRight",
        "Control",
        163,
        without_location = 17,
        location = 2
    ),
    key!("PrintScreen", "PrintScreen", 44),
    key!("ScrollLock", "ScrollLock", 145),
    key!("Pause", "Pause", 19),
    key!("PageUp", "PageUp", 33),
    key!("PageDown", "PageDown", 34),
    key!("Insert", "Insert", 45),
    key!("Delete", "Delete", 46),
    key!("Home", "Home", 36),
    key!("End", "End", 35),
    key!("ArrowLeft", "ArrowLeft", 37),
    key!("ArrowUp", "ArrowUp", 38),
    key!("ArrowRight", "ArrowRight", 39),
    key!("ArrowDown", "ArrowDown", 40),
    key!("AudioVolumeMute", "AudioVolumeMute", 173),
    key!("AudioVolumeDown", "AudioVolumeDown", 174),
    key!("AudioVolumeUp", "AudioVolumeUp", 175),
    key!("MediaTrackNext", "MediaTrackNext", 176),
    key!("MediaTrackPrevious", "MediaTrackPrevious", 177),
    key!("MediaPlayPause", "MediaPlayPause", 179),
    key!("NumLock", "NumLock", 144),
    key!("NumpadDivide", "/", 111, location = 3),
    key!("NumpadMultiply", "*", 106, location = 3),
    key!("NumpadSubtract", "-", 109, location = 3),
    key!(
        "Numpad7",
        "Home",
        36,
        shifted = "7",
        shifted_code = 103,
        location = 3
    ),
    key!(
        "Numpad8",
        "ArrowUp",
        38,
        shifted = "8",
        shifted_code = 104,
        location = 3
    ),
    key!(
        "Numpad9",
        "PageUp",
        33,
        shifted = "9",
        shifted_code = 105,
        location = 3
    ),
    key!(
        "Numpad4",
        "ArrowLeft",
        37,
        shifted = "4",
        shifted_code = 100,
        location = 3
    ),
    key!(
        "Numpad5",
        "Clear",
        12,
        shifted = "5",
        shifted_code = 101,
        location = 3
    ),
    key!(
        "Numpad6",
        "ArrowRight",
        39,
        shifted = "6",
        shifted_code = 102,
        location = 3
    ),
    key!("NumpadAdd", "+", 107, location = 3),
    key!(
        "Numpad1",
        "End",
        35,
        shifted = "1",
        shifted_code = 97,
        location = 3
    ),
    key!(
        "Numpad2",
        "ArrowDown",
        40,
        shifted = "2",
        shifted_code = 98,
        location = 3
    ),
    key!(
        "Numpad3",
        "PageDown",
        34,
        shifted = "3",
        shifted_code = 99,
        location = 3
    ),
    key!(
        "Numpad0",
        "Insert",
        45,
        shifted = "0",
        shifted_code = 96,
        location = 3
    ),
    key!(
        "NumpadDecimal",
        "\0",
        46,
        shifted = ".",
        shifted_code = 110,
        location = 3
    ),
    key!("NumpadEnter", "Enter", 13, text = "\r", location = 3),
];

fn description(definition: &KeyDefinition) -> KeyDescription {
    let text = definition.text.unwrap_or_else(|| {
        if definition.key.chars().count() == 1 {
            definition.key
        } else {
            ""
        }
    });
    KeyDescription {
        code: definition.code,
        key: definition.key,
        key_code: definition.key_code,
        key_code_without_location: definition
            .key_code_without_location
            .unwrap_or(definition.key_code),
        text,
        location: definition.location,
    }
}

fn shifted_description(definition: &KeyDefinition) -> Option<KeyDescription> {
    let shifted = definition.shifted.as_ref()?;
    Some(KeyDescription {
        code: definition.code,
        key: shifted.key,
        key_code: shifted.key_code.unwrap_or(definition.key_code),
        key_code_without_location: definition
            .key_code_without_location
            .unwrap_or(definition.key_code),
        text: shifted.key,
        location: definition.location,
    })
}

pub fn lookup(key: &str, shift: bool) -> Option<KeyDescription> {
    let alias_code = match key {
        "Shift" => Some("ShiftLeft"),
        "Control" => Some("ControlLeft"),
        "Alt" => Some("AltLeft"),
        "Meta" => Some("MetaLeft"),
        "\n" | "\r" => Some("Enter"),
        _ => None,
    };
    if let Some(code) = alias_code {
        return US_KEYBOARD_LAYOUT
            .iter()
            .find(|definition| definition.code == code)
            .map(description);
    }
    if let Some(definition) = US_KEYBOARD_LAYOUT
        .iter()
        .find(|definition| definition.code == key)
    {
        if shift {
            if let Some(shifted) = shifted_description(definition) {
                return Some(shifted);
            }
        }
        return Some(description(definition));
    }
    for definition in US_KEYBOARD_LAYOUT
        .iter()
        .filter(|definition| definition.location == 0)
    {
        if definition.key == key && definition.key.chars().count() == 1 {
            return Some(description(definition));
        }
        if let Some(shifted) = shifted_description(definition) {
            if shifted.key == key {
                return Some(shifted);
            }
        }
    }
    None
}

pub fn has(key: &str) -> bool {
    lookup(key, false).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_digits_and_shifted_symbols_resolve() {
        assert_eq!(lookup("a", false).unwrap().code, "KeyA");
        assert_eq!(lookup("KeyA", true).unwrap().key, "A");
        assert_eq!(lookup("1", false).unwrap().code, "Digit1");
        assert_eq!(lookup("Digit1", true).unwrap().key, "!");
        assert_eq!(lookup("!", false).unwrap().code, "Digit1");
    }

    #[test]
    fn control_and_navigation_keys_resolve() {
        assert_eq!(lookup("Enter", false).unwrap().text, "\r");
        assert_eq!(lookup("Tab", false).unwrap().key_code, 9);
        assert_eq!(lookup("Escape", false).unwrap().key_code, 27);
        assert_eq!(lookup("ArrowLeft", false).unwrap().key_code, 37);
        assert_eq!(lookup("F12", false).unwrap().key_code, 123);
    }

    #[test]
    fn numpad_shift_changes_key_and_key_code() {
        let shifted = lookup("Numpad7", true).unwrap();
        assert_eq!(shifted.key, "7");
        assert_eq!(shifted.key_code, 103);
        assert_eq!(shifted.location, KEYPAD_LOCATION);
    }
}
