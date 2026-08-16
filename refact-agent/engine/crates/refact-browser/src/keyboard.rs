use std::collections::HashSet;
use std::thread;
use std::time::Duration;

use headless_chrome::Tab;
use headless_chrome::protocol::cdp::Input;

use crate::us_keyboard_layout::{self, KEYPAD_LOCATION, KeyDescription};

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum KeyboardModifier {
    Alt,
    Control,
    Meta,
    Shift,
}

impl KeyboardModifier {
    fn from_key(key: &str) -> Option<Self> {
        match key {
            "Alt" => Some(Self::Alt),
            "Control" => Some(Self::Control),
            "Meta" => Some(Self::Meta),
            "Shift" => Some(Self::Shift),
            _ => None,
        }
    }
}

pub fn modifier_bitmask(modifiers: &HashSet<KeyboardModifier>) -> u32 {
    modifiers.iter().fold(0, |mask, modifier| {
        mask | match modifier {
            KeyboardModifier::Alt => 1,
            KeyboardModifier::Control => 2,
            KeyboardModifier::Meta => 4,
            KeyboardModifier::Shift => 8,
        }
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyEventType {
    KeyDown,
    RawKeyDown,
    KeyUp,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyEventPayload {
    pub event_type: KeyEventType,
    pub modifiers: u32,
    pub windows_virtual_key_code: u32,
    pub code: String,
    pub key: String,
    pub text: Option<String>,
    pub unmodified_text: Option<String>,
    pub auto_repeat: Option<bool>,
    pub location: u32,
    pub is_keypad: Option<bool>,
    pub commands: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyboardDispatch {
    Key(KeyEventPayload),
    InsertText(String),
}

pub trait KeyboardDispatcher {
    fn dispatch(&mut self, event: KeyboardDispatch) -> Result<(), String>;
}

pub struct CdpKeyboardDispatcher<'a> {
    tab: &'a Tab,
}

impl<'a> CdpKeyboardDispatcher<'a> {
    pub fn new(tab: &'a Tab) -> Self {
        Self { tab }
    }
}

impl KeyboardDispatcher for CdpKeyboardDispatcher<'_> {
    fn dispatch(&mut self, event: KeyboardDispatch) -> Result<(), String> {
        match event {
            KeyboardDispatch::Key(payload) => {
                let event_type = match payload.event_type {
                    KeyEventType::KeyDown => Input::DispatchKeyEventTypeOption::KeyDown,
                    KeyEventType::RawKeyDown => Input::DispatchKeyEventTypeOption::RawKeyDown,
                    KeyEventType::KeyUp => Input::DispatchKeyEventTypeOption::KeyUp,
                };
                self.tab
                    .call_method(Input::DispatchKeyEvent {
                        Type: event_type,
                        modifiers: Some(payload.modifiers),
                        timestamp: None,
                        text: payload.text,
                        unmodified_text: payload.unmodified_text,
                        key_identifier: None,
                        code: Some(payload.code),
                        key: Some(payload.key),
                        windows_virtual_key_code: Some(payload.windows_virtual_key_code),
                        native_virtual_key_code: None,
                        auto_repeat: payload.auto_repeat,
                        is_keypad: payload.is_keypad,
                        is_system_key: None,
                        location: Some(payload.location),
                        commands: payload.commands,
                    })
                    .map_err(|error| format!("Failed to dispatch browser key event: {error}"))?;
            }
            KeyboardDispatch::InsertText(text) => {
                self.tab
                    .call_method(Input::InsertText { text })
                    .map_err(|error| format!("Failed to insert browser text: {error}"))?;
            }
        }
        Ok(())
    }
}

pub struct Keyboard<D> {
    dispatcher: D,
    pressed_modifiers: HashSet<KeyboardModifier>,
    pressed_keys: HashSet<&'static str>,
}

impl<D: KeyboardDispatcher> Keyboard<D> {
    pub fn new(dispatcher: D) -> Self {
        Self {
            dispatcher,
            pressed_modifiers: HashSet::new(),
            pressed_keys: HashSet::new(),
        }
    }

    pub fn down(&mut self, key: &str) -> Result<(), String> {
        let description = self.key_description(key)?;
        let auto_repeat = self.pressed_keys.contains(description.code);
        self.pressed_keys.insert(description.code);
        if let Some(modifier) = KeyboardModifier::from_key(description.key) {
            self.pressed_modifiers.insert(modifier);
        }
        self.dispatcher
            .dispatch(KeyboardDispatch::Key(key_down_payload(
                &description,
                modifier_bitmask(&self.pressed_modifiers),
                auto_repeat,
            )))
    }

    pub fn up(&mut self, key: &str) -> Result<(), String> {
        let description = self.key_description(key)?;
        if let Some(modifier) = KeyboardModifier::from_key(description.key) {
            self.pressed_modifiers.remove(&modifier);
        }
        self.pressed_keys.remove(description.code);
        self.dispatcher
            .dispatch(KeyboardDispatch::Key(key_up_payload(
                &description,
                modifier_bitmask(&self.pressed_modifiers),
            )))
    }

    pub fn press(&mut self, key: &str, delay: Option<Duration>) -> Result<(), String> {
        let tokens = split_key(key);
        let mut pressed = Vec::new();
        for token in &tokens {
            pressed.push(token.as_str());
            if let Err(error) = self.down(token) {
                self.release_pressed(&pressed);
                return Err(error);
            }
        }
        if let Some(delay) = delay {
            thread::sleep(delay);
        }
        let mut first_error = None;
        for token in pressed.into_iter().rev() {
            if let Err(error) = self.up(token) {
                first_error.get_or_insert(error);
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    pub fn type_text(&mut self, text: &str, delay: Option<Duration>) -> Result<(), String> {
        for character in text.chars() {
            let character = character.to_string();
            if us_keyboard_layout::has(&character) {
                self.press(&character, delay)?;
            } else {
                if let Some(delay) = delay {
                    thread::sleep(delay);
                }
                self.insert_text(&character)?;
            }
        }
        Ok(())
    }

    pub fn press_sequentially(
        &mut self,
        text: &str,
        delay: Option<Duration>,
    ) -> Result<(), String> {
        self.type_text(text, delay)
    }

    pub fn insert_text(&mut self, text: &str) -> Result<(), String> {
        self.dispatcher
            .dispatch(KeyboardDispatch::InsertText(text.to_string()))
    }

    fn key_description(&self, key: &str) -> Result<KeyDescription, String> {
        let shift = self.pressed_modifiers.contains(&KeyboardModifier::Shift);
        let mut description = us_keyboard_layout::lookup(key, shift)
            .ok_or_else(|| format!("Unknown key: \"{key}\""))?;
        let only_shift = self.pressed_modifiers.len() == 1 && shift;
        if !self.pressed_modifiers.is_empty() && !only_shift {
            description.text = "";
        }
        Ok(description)
    }

    fn release_pressed(&mut self, pressed: &[&str]) {
        for key in pressed.iter().rev() {
            let _ = self.up(key);
        }
        self.pressed_modifiers.clear();
        self.pressed_keys.clear();
    }
}

fn split_key(key: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let mut building = String::new();
    for character in key.chars() {
        if character == '+' && !building.is_empty() {
            keys.push(std::mem::take(&mut building));
        } else {
            building.push(character);
        }
    }
    keys.push(building);
    keys
}

fn key_down_payload(
    description: &KeyDescription,
    modifiers: u32,
    auto_repeat: bool,
) -> KeyEventPayload {
    let text = description.text.to_string();
    KeyEventPayload {
        event_type: if text.is_empty() {
            KeyEventType::RawKeyDown
        } else {
            KeyEventType::KeyDown
        },
        modifiers,
        windows_virtual_key_code: description.key_code_without_location,
        code: description.code.to_string(),
        key: description.key.to_string(),
        text: Some(text.clone()),
        unmodified_text: Some(text),
        auto_repeat: Some(auto_repeat),
        location: description.location,
        is_keypad: Some(description.location == KEYPAD_LOCATION),
        commands: Some(Vec::new()),
    }
}

fn key_up_payload(description: &KeyDescription, modifiers: u32) -> KeyEventPayload {
    KeyEventPayload {
        event_type: KeyEventType::KeyUp,
        modifiers,
        windows_virtual_key_code: description.key_code_without_location,
        code: description.code.to_string(),
        key: description.key.to_string(),
        text: None,
        unmodified_text: None,
        auto_repeat: None,
        location: description.location,
        is_keypad: None,
        commands: None,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Clone, Default)]
    struct RecordingDispatcher {
        events: Arc<Mutex<Vec<KeyboardDispatch>>>,
        fail_at: Option<usize>,
    }

    impl KeyboardDispatcher for RecordingDispatcher {
        fn dispatch(&mut self, event: KeyboardDispatch) -> Result<(), String> {
            let mut events = self.events.lock().unwrap();
            events.push(event);
            if self.fail_at == Some(events.len()) {
                return Err("dispatch failed".to_string());
            }
            Ok(())
        }
    }

    fn key_event(
        event_type: KeyEventType,
        modifiers: u32,
        code: &str,
        key: &str,
        text: Option<&str>,
        key_code: u32,
    ) -> KeyboardDispatch {
        let down = event_type != KeyEventType::KeyUp;
        KeyboardDispatch::Key(KeyEventPayload {
            event_type,
            modifiers,
            windows_virtual_key_code: key_code,
            code: code.to_string(),
            key: key.to_string(),
            text: text.map(str::to_string),
            unmodified_text: text.map(str::to_string),
            auto_repeat: down.then_some(false),
            location: if code.ends_with("Left") { 1 } else { 0 },
            is_keypad: down.then_some(false),
            commands: down.then(Vec::new),
        })
    }

    #[test]
    fn modifier_bitmask_matches_cdp() {
        let modifiers = HashSet::from([
            KeyboardModifier::Alt,
            KeyboardModifier::Control,
            KeyboardModifier::Meta,
            KeyboardModifier::Shift,
        ]);
        assert_eq!(modifier_bitmask(&modifiers), 15);
    }

    #[test]
    fn press_orders_modifiers_key_and_reverse_releases() {
        let dispatcher = RecordingDispatcher::default();
        let events = dispatcher.events.clone();
        let mut keyboard = Keyboard::new(dispatcher);
        keyboard.press("Control+Shift+T", None).unwrap();
        assert_eq!(
            *events.lock().unwrap(),
            vec![
                key_event(
                    KeyEventType::RawKeyDown,
                    2,
                    "ControlLeft",
                    "Control",
                    Some(""),
                    17,
                ),
                key_event(
                    KeyEventType::RawKeyDown,
                    10,
                    "ShiftLeft",
                    "Shift",
                    Some(""),
                    16,
                ),
                key_event(KeyEventType::RawKeyDown, 10, "KeyT", "T", Some(""), 84,),
                key_event(KeyEventType::KeyUp, 10, "KeyT", "T", None, 84),
                key_event(KeyEventType::KeyUp, 2, "ShiftLeft", "Shift", None, 16),
                key_event(KeyEventType::KeyUp, 0, "ControlLeft", "Control", None, 17),
            ]
        );
    }

    #[test]
    fn shift_uses_shifted_key_and_text() {
        let dispatcher = RecordingDispatcher::default();
        let events = dispatcher.events.clone();
        let mut keyboard = Keyboard::new(dispatcher);
        keyboard.press("Shift+Digit1", None).unwrap();
        let events = events.lock().unwrap();
        assert_eq!(
            events[1],
            key_event(KeyEventType::KeyDown, 8, "Digit1", "!", Some("!"), 49)
        );
    }

    #[test]
    fn type_uses_key_events_then_insert_text_for_unicode() {
        let dispatcher = RecordingDispatcher::default();
        let events = dispatcher.events.clone();
        let mut keyboard = Keyboard::new(dispatcher);
        keyboard.type_text("aé", None).unwrap();
        assert_eq!(
            *events.lock().unwrap(),
            vec![
                key_event(KeyEventType::KeyDown, 0, "KeyA", "a", Some("a"), 65),
                key_event(KeyEventType::KeyUp, 0, "KeyA", "a", None, 65),
                KeyboardDispatch::InsertText("é".to_string()),
            ]
        );
    }

    #[test]
    fn insert_text_emits_no_key_events() {
        let dispatcher = RecordingDispatcher::default();
        let events = dispatcher.events.clone();
        let mut keyboard = Keyboard::new(dispatcher);
        keyboard.insert_text("🙂").unwrap();
        assert_eq!(
            *events.lock().unwrap(),
            vec![KeyboardDispatch::InsertText("🙂".to_string())]
        );
    }

    #[test]
    fn press_releases_modifiers_after_dispatch_error() {
        let dispatcher = RecordingDispatcher {
            fail_at: Some(3),
            ..RecordingDispatcher::default()
        };
        let events = dispatcher.events.clone();
        let mut keyboard = Keyboard::new(dispatcher);
        assert_eq!(
            keyboard.press("Control+Shift+T", None).unwrap_err(),
            "dispatch failed"
        );
        let events = events.lock().unwrap();
        assert!(events.iter().any(|event| {
            matches!(
                event,
                KeyboardDispatch::Key(KeyEventPayload {
                    event_type: KeyEventType::KeyUp,
                    key,
                    ..
                }) if key == "Control"
            )
        }));
        assert!(keyboard.pressed_modifiers.is_empty());
        assert!(keyboard.pressed_keys.is_empty());
    }

    #[test]
    fn unknown_key_uses_playwright_error_text() {
        let mut keyboard = Keyboard::new(RecordingDispatcher::default());
        assert_eq!(
            keyboard.press("Hyper", None).unwrap_err(),
            "Unknown key: \"Hyper\""
        );
    }
}
