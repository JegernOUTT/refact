use std::collections::HashSet;
use std::error::Error;
use std::fmt::{Display, Formatter};

use headless_chrome::Tab;
use headless_chrome::protocol::cdp::{DOM, Input, Page, Runtime};
use serde_json::json;

use crate::{ElementHandle, Keyboard, KeyboardDispatcher};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MainFrameCssPoint {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MainFrameCssViewport {
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContentQuad(pub [MainFrameCssPoint; 4]);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MouseError {
    NoQuads,
    OutsideViewport,
    InvalidSteps,
    Protocol(String),
}

impl Display for MouseError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoQuads => formatter.write_str("Element has no content quads"),
            Self::OutsideViewport => formatter.write_str("Element is outside of the viewport"),
            Self::InvalidSteps => formatter.write_str("Mouse move steps must be greater than zero"),
            Self::Protocol(message) => formatter.write_str(message),
        }
    }
}

impl Error for MouseError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollStrategy {
    Protocol,
    End,
    Center,
    Start,
}

impl ScrollStrategy {
    pub fn for_attempt(attempt: usize) -> Self {
        const STRATEGIES: [ScrollStrategy; 4] = [
            ScrollStrategy::Protocol,
            ScrollStrategy::End,
            ScrollStrategy::Center,
            ScrollStrategy::Start,
        ];
        STRATEGIES[attempt % STRATEGIES.len()]
    }

    pub fn advance(self) -> Self {
        match self {
            Self::Protocol => Self::End,
            Self::End => Self::Center,
            Self::Center => Self::Start,
            Self::Start => Self::Protocol,
        }
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum MouseButton {
    None,
    Left,
    Middle,
    Right,
}

impl MouseButton {
    fn bit(self) -> u32 {
        match self {
            Self::None => 0,
            Self::Left => 1,
            Self::Right => 2,
            Self::Middle => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseEventType {
    Moved,
    Pressed,
    Released,
    Wheel,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MouseEventPayload {
    pub event_type: MouseEventType,
    pub x: f64,
    pub y: f64,
    pub button: Option<MouseButton>,
    pub buttons: Option<u32>,
    pub modifiers: u32,
    pub click_count: Option<u32>,
    pub force: Option<f64>,
    pub delta_x: Option<f64>,
    pub delta_y: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TouchEventType {
    Start,
    End,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TouchEventPayload {
    pub event_type: TouchEventType,
    pub touch_points: Vec<MainFrameCssPoint>,
    pub modifiers: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MouseDispatch {
    Mouse(MouseEventPayload),
    Touch(TouchEventPayload),
}

pub trait MouseDispatcher {
    fn dispatch(&mut self, event: MouseDispatch) -> Result<(), String>;
}

#[derive(Clone, Debug)]
pub struct MouseState {
    position: MainFrameCssPoint,
    last_button: Option<MouseButton>,
    buttons: HashSet<MouseButton>,
}

impl Default for MouseState {
    fn default() -> Self {
        Self {
            position: MainFrameCssPoint::default(),
            last_button: Some(MouseButton::None),
            buttons: HashSet::new(),
        }
    }
}

impl MouseState {
    pub fn reset_buttons(&mut self) {
        self.last_button = None;
        self.buttons.clear();
    }
}

pub struct CdpMouseDispatcher<'a> {
    tab: &'a Tab,
}

impl<'a> CdpMouseDispatcher<'a> {
    pub fn new(tab: &'a Tab) -> Self {
        Self { tab }
    }

    pub fn clickable_point(&self, handle: &ElementHandle) -> Result<MainFrameCssPoint, MouseError> {
        let quads = self
            .tab
            .call_method(DOM::GetContentQuads {
                node_id: None,
                backend_node_id: None,
                object_id: Some(handle.object_id.clone()),
            })
            .map_err(|error| {
                MouseError::Protocol(format!(
                    "Failed to read browser element content quads: {error}"
                ))
            })?
            .quads;
        let quads = quads
            .into_iter()
            .map(content_quad_from_cdp)
            .collect::<Result<Vec<_>, _>>()?;
        let metrics = self
            .tab
            .call_method(Page::GetLayoutMetrics(None))
            .map_err(|error| {
                MouseError::Protocol(format!("Failed to read browser viewport metrics: {error}"))
            })?;
        clickable_point_from_quads(
            &quads,
            MainFrameCssViewport {
                width: f64::from(metrics.css_layout_viewport.client_width),
                height: f64::from(metrics.css_layout_viewport.client_height),
            },
        )
    }

    pub fn scroll_into_view(
        &self,
        handle: &ElementHandle,
        strategy: ScrollStrategy,
    ) -> Result<(), MouseError> {
        if strategy == ScrollStrategy::Protocol {
            self.tab
                .call_method(DOM::ScrollIntoViewIfNeeded {
                    node_id: None,
                    backend_node_id: None,
                    object_id: Some(handle.object_id.clone()),
                    rect: None,
                })
                .map_err(|error| {
                    MouseError::Protocol(format!(
                        "Failed to scroll browser element into view: {error}"
                    ))
                })?;
            return Ok(());
        }
        let alignment = match strategy {
            ScrollStrategy::End => "end",
            ScrollStrategy::Center => "center",
            ScrollStrategy::Start => "start",
            ScrollStrategy::Protocol => unreachable!(),
        };
        let result = self
            .tab
            .call_method(Runtime::CallFunctionOn {
                function_declaration:
                    "function(options) { this.scrollIntoView(options); return true; }".to_string(),
                object_id: Some(handle.object_id.clone()),
                arguments: Some(vec![Runtime::CallArgument {
                    value: Some(json!({"block": alignment, "inline": alignment})),
                    unserializable_value: None,
                    object_id: None,
                }]),
                silent: None,
                return_by_value: Some(true),
                generate_preview: None,
                user_gesture: Some(true),
                await_promise: Some(true),
                execution_context_id: None,
                object_group: None,
                throw_on_side_effect: None,
                unique_context_id: None,
                serialization_options: None,
            })
            .map_err(|error| {
                MouseError::Protocol(format!("Failed to align browser element in view: {error}"))
            })?;
        if let Some(exception) = result.exception_details {
            return Err(MouseError::Protocol(
                exception
                    .exception
                    .and_then(|exception| exception.description)
                    .unwrap_or(exception.text),
            ));
        }
        Ok(())
    }
}

impl MouseDispatcher for CdpMouseDispatcher<'_> {
    fn dispatch(&mut self, event: MouseDispatch) -> Result<(), String> {
        match event {
            MouseDispatch::Mouse(payload) => {
                self.tab
                    .call_method(Input::DispatchMouseEvent {
                        Type: match payload.event_type {
                            MouseEventType::Moved => {
                                Input::DispatchMouseEventTypeOption::MouseMoved
                            }
                            MouseEventType::Pressed => {
                                Input::DispatchMouseEventTypeOption::MousePressed
                            }
                            MouseEventType::Released => {
                                Input::DispatchMouseEventTypeOption::MouseReleased
                            }
                            MouseEventType::Wheel => {
                                Input::DispatchMouseEventTypeOption::MouseWheel
                            }
                        },
                        x: payload.x,
                        y: payload.y,
                        modifiers: Some(payload.modifiers),
                        timestamp: None,
                        button: payload.button.map(cdp_mouse_button),
                        buttons: payload.buttons,
                        click_count: payload.click_count,
                        force: payload.force,
                        tangential_pressure: None,
                        tilt_x: None,
                        tilt_y: None,
                        twist: None,
                        delta_x: payload.delta_x,
                        delta_y: payload.delta_y,
                        pointer_Type: None,
                    })
                    .map_err(|error| format!("Failed to dispatch browser mouse event: {error}"))?;
            }
            MouseDispatch::Touch(payload) => {
                self.tab
                    .call_method(Input::DispatchTouchEvent {
                        Type: match payload.event_type {
                            TouchEventType::Start => {
                                Input::DispatchTouchEventTypeOption::TouchStart
                            }
                            TouchEventType::End => Input::DispatchTouchEventTypeOption::TouchEnd,
                        },
                        touch_points: payload
                            .touch_points
                            .into_iter()
                            .map(|point| Input::TouchPoint {
                                x: point.x,
                                y: point.y,
                                radius_x: None,
                                radius_y: None,
                                rotation_angle: None,
                                force: None,
                                tangential_pressure: None,
                                tilt_x: None,
                                tilt_y: None,
                                twist: None,
                                id: None,
                            })
                            .collect(),
                        modifiers: Some(payload.modifiers),
                        timestamp: None,
                    })
                    .map_err(|error| format!("Failed to dispatch browser touch event: {error}"))?;
            }
        }
        Ok(())
    }
}

pub struct Mouse<'a, D, K> {
    dispatcher: D,
    keyboard: &'a Keyboard<K>,
    state: MouseState,
}

impl<'a, D: MouseDispatcher, K: KeyboardDispatcher> Mouse<'a, D, K> {
    pub fn new(dispatcher: D, keyboard: &'a Keyboard<K>) -> Self {
        Self::from_state(dispatcher, keyboard, MouseState::default())
    }

    pub fn from_state(dispatcher: D, keyboard: &'a Keyboard<K>, state: MouseState) -> Self {
        Self {
            dispatcher,
            keyboard,
            state,
        }
    }

    pub fn position(&self) -> MainFrameCssPoint {
        self.state.position
    }

    pub fn state(&self) -> MouseState {
        self.state.clone()
    }

    pub fn reset_buttons(&mut self) {
        self.state.reset_buttons();
    }

    pub fn move_to(&mut self, x: f64, y: f64, steps: usize) -> Result<(), MouseError> {
        if steps == 0 {
            return Err(MouseError::InvalidSteps);
        }
        let from = self.state.position;
        self.state.position = MainFrameCssPoint { x, y };
        for step in 1..=steps {
            let progress = step as f64 / steps as f64;
            self.dispatch_mouse(MouseEventPayload {
                event_type: MouseEventType::Moved,
                x: from.x + (x - from.x) * progress,
                y: from.y + (y - from.y) * progress,
                button: self.state.last_button,
                buttons: Some(buttons_bitmask(&self.state.buttons)),
                modifiers: self.keyboard.modifier_bitmask(),
                click_count: None,
                force: Some(if self.state.buttons.is_empty() {
                    0.0
                } else {
                    0.5
                }),
                delta_x: None,
                delta_y: None,
            })?;
        }
        Ok(())
    }

    pub fn hover(&mut self, x: f64, y: f64) -> Result<(), MouseError> {
        self.move_to(x, y, 1)
    }

    pub fn down(&mut self, button: MouseButton, click_count: u32) -> Result<(), MouseError> {
        self.state.last_button = Some(button);
        self.state.buttons.insert(button);
        self.dispatch_mouse(MouseEventPayload {
            event_type: MouseEventType::Pressed,
            x: self.state.position.x,
            y: self.state.position.y,
            button: Some(button),
            buttons: Some(buttons_bitmask(&self.state.buttons)),
            modifiers: self.keyboard.modifier_bitmask(),
            click_count: Some(click_count),
            force: Some(0.5),
            delta_x: None,
            delta_y: None,
        })
    }

    pub fn up(&mut self, button: MouseButton, click_count: u32) -> Result<(), MouseError> {
        self.state.last_button = None;
        self.state.buttons.remove(&button);
        self.dispatch_mouse(MouseEventPayload {
            event_type: MouseEventType::Released,
            x: self.state.position.x,
            y: self.state.position.y,
            button: Some(button),
            buttons: Some(buttons_bitmask(&self.state.buttons)),
            modifiers: self.keyboard.modifier_bitmask(),
            click_count: Some(click_count),
            force: None,
            delta_x: None,
            delta_y: None,
        })
    }

    pub fn click(&mut self, x: f64, y: f64, button: MouseButton) -> Result<(), MouseError> {
        self.move_to(x, y, 1)?;
        self.down(button, 1)?;
        self.up(button, 1)
    }

    pub fn dblclick(&mut self, x: f64, y: f64, button: MouseButton) -> Result<(), MouseError> {
        self.move_to(x, y, 1)?;
        self.down(button, 1)?;
        self.up(button, 1)?;
        self.down(button, 2)?;
        self.up(button, 2)
    }

    pub fn wheel(&mut self, delta_x: f64, delta_y: f64) -> Result<(), MouseError> {
        self.dispatch_mouse(MouseEventPayload {
            event_type: MouseEventType::Wheel,
            x: self.state.position.x,
            y: self.state.position.y,
            button: None,
            buttons: None,
            modifiers: self.keyboard.modifier_bitmask(),
            click_count: None,
            force: None,
            delta_x: Some(delta_x),
            delta_y: Some(delta_y),
        })
    }

    pub fn tap(&mut self, x: f64, y: f64) -> Result<(), MouseError> {
        let modifiers = self.keyboard.modifier_bitmask();
        self.dispatch_touch(TouchEventPayload {
            event_type: TouchEventType::Start,
            touch_points: vec![MainFrameCssPoint { x, y }],
            modifiers,
        })?;
        self.dispatch_touch(TouchEventPayload {
            event_type: TouchEventType::End,
            touch_points: Vec::new(),
            modifiers,
        })
    }

    fn dispatch_mouse(&mut self, payload: MouseEventPayload) -> Result<(), MouseError> {
        self.dispatcher
            .dispatch(MouseDispatch::Mouse(payload))
            .map_err(MouseError::Protocol)
    }

    fn dispatch_touch(&mut self, payload: TouchEventPayload) -> Result<(), MouseError> {
        self.dispatcher
            .dispatch(MouseDispatch::Touch(payload))
            .map_err(MouseError::Protocol)
    }
}

pub fn clickable_point_from_quads(
    quads: &[ContentQuad],
    viewport: MainFrameCssViewport,
) -> Result<MainFrameCssPoint, MouseError> {
    if quads.is_empty() {
        return Err(MouseError::NoQuads);
    }
    quads
        .iter()
        .map(|quad| intersect_quad_with_viewport(*quad, viewport))
        .find(|quad| quad_area(*quad) > 0.99)
        .map(quad_center)
        .ok_or(MouseError::OutsideViewport)
}

fn content_quad_from_cdp(values: Vec<f64>) -> Result<ContentQuad, MouseError> {
    let values: [f64; 8] = values.try_into().map_err(|values: Vec<f64>| {
        MouseError::Protocol(format!(
            "Browser content quad must contain 8 coordinates, got {}",
            values.len()
        ))
    })?;
    Ok(ContentQuad([
        MainFrameCssPoint {
            x: values[0],
            y: values[1],
        },
        MainFrameCssPoint {
            x: values[2],
            y: values[3],
        },
        MainFrameCssPoint {
            x: values[4],
            y: values[5],
        },
        MainFrameCssPoint {
            x: values[6],
            y: values[7],
        },
    ]))
}

fn intersect_quad_with_viewport(quad: ContentQuad, viewport: MainFrameCssViewport) -> ContentQuad {
    ContentQuad(quad.0.map(|point| MainFrameCssPoint {
        x: point.x.clamp(0.0, viewport.width),
        y: point.y.clamp(0.0, viewport.height),
    }))
}

fn quad_area(quad: ContentQuad) -> f64 {
    let mut area = 0.0;
    for index in 0..quad.0.len() {
        let first = quad.0[index];
        let second = quad.0[(index + 1) % quad.0.len()];
        area += (first.x * second.y - second.x * first.y) / 2.0;
    }
    area.abs()
}

fn quad_center(quad: ContentQuad) -> MainFrameCssPoint {
    MainFrameCssPoint {
        x: quad.0.iter().map(|point| point.x).sum::<f64>() / 4.0,
        y: quad.0.iter().map(|point| point.y).sum::<f64>() / 4.0,
    }
}

fn buttons_bitmask(buttons: &HashSet<MouseButton>) -> u32 {
    buttons.iter().fold(0, |mask, button| mask | button.bit())
}

fn cdp_mouse_button(button: MouseButton) -> Input::MouseButton {
    match button {
        MouseButton::None => Input::MouseButton::None,
        MouseButton::Left => Input::MouseButton::Left,
        MouseButton::Middle => Input::MouseButton::Middle,
        MouseButton::Right => Input::MouseButton::Right,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use crate::{KeyboardDispatch, KeyboardDispatcher};

    use super::*;

    #[derive(Clone, Default)]
    struct RecordingMouseDispatcher {
        events: Arc<Mutex<Vec<MouseDispatch>>>,
    }

    impl MouseDispatcher for RecordingMouseDispatcher {
        fn dispatch(&mut self, event: MouseDispatch) -> Result<(), String> {
            self.events.lock().unwrap().push(event);
            Ok(())
        }
    }

    #[derive(Default)]
    struct RecordingKeyboardDispatcher;

    impl KeyboardDispatcher for RecordingKeyboardDispatcher {
        fn dispatch(&mut self, _event: KeyboardDispatch) -> Result<(), String> {
            Ok(())
        }
    }

    fn quad(points: [(f64, f64); 4]) -> ContentQuad {
        ContentQuad(points.map(|(x, y)| MainFrameCssPoint { x, y }))
    }

    fn viewport() -> MainFrameCssViewport {
        MainFrameCssViewport {
            width: 100.0,
            height: 80.0,
        }
    }

    fn mouse_payload(
        event_type: MouseEventType,
        x: f64,
        y: f64,
        button: Option<MouseButton>,
        buttons: Option<u32>,
        click_count: Option<u32>,
        force: Option<f64>,
    ) -> MouseDispatch {
        MouseDispatch::Mouse(MouseEventPayload {
            event_type,
            x,
            y,
            button,
            buttons,
            modifiers: 0,
            click_count,
            force,
            delta_x: None,
            delta_y: None,
        })
    }

    #[test]
    fn clickable_point_selects_first_quad_larger_than_threshold() {
        let quads = [
            quad([(0.0, 0.0), (0.99, 0.0), (0.99, 1.0), (0.0, 1.0)]),
            quad([(10.0, 20.0), (14.0, 20.0), (14.0, 24.0), (10.0, 24.0)]),
            quad([(30.0, 40.0), (34.0, 40.0), (34.0, 44.0), (30.0, 44.0)]),
        ];
        assert_eq!(
            clickable_point_from_quads(&quads, viewport()).unwrap(),
            MainFrameCssPoint { x: 12.0, y: 22.0 }
        );
    }

    #[test]
    fn clickable_point_rejects_tiny_and_degenerate_quads() {
        let quads = [
            quad([(1.0, 1.0), (1.5, 1.0), (1.5, 1.5), (1.0, 1.5)]),
            quad([(2.0, 2.0), (3.0, 2.0), (4.0, 2.0), (5.0, 2.0)]),
        ];
        assert_eq!(
            clickable_point_from_quads(&quads, viewport()),
            Err(MouseError::OutsideViewport)
        );
        assert_eq!(
            clickable_point_from_quads(&[], viewport()),
            Err(MouseError::NoQuads)
        );
    }

    #[test]
    fn clickable_point_clips_quad_to_viewport() {
        let quads = [quad([(-4.0, -2.0), (6.0, -2.0), (6.0, 4.0), (-4.0, 4.0)])];
        assert_eq!(
            clickable_point_from_quads(&quads, viewport()).unwrap(),
            MainFrameCssPoint { x: 3.0, y: 2.0 }
        );
    }

    #[test]
    fn move_emits_each_interpolated_point() {
        let dispatcher = RecordingMouseDispatcher::default();
        let events = dispatcher.events.clone();
        let keyboard = Keyboard::new(RecordingKeyboardDispatcher);
        let mut mouse = Mouse::new(dispatcher, &keyboard);
        mouse.move_to(10.0, 20.0, 5).unwrap();
        assert_eq!(
            *events.lock().unwrap(),
            (1..=5)
                .map(|step| {
                    mouse_payload(
                        MouseEventType::Moved,
                        step as f64 * 2.0,
                        step as f64 * 4.0,
                        Some(MouseButton::None),
                        Some(0),
                        None,
                        Some(0.0),
                    )
                })
                .collect::<Vec<_>>()
        );
        assert_eq!(mouse.position(), MainFrameCssPoint { x: 10.0, y: 20.0 });
    }

    #[test]
    fn click_emits_move_press_release_sequence() {
        let dispatcher = RecordingMouseDispatcher::default();
        let events = dispatcher.events.clone();
        let keyboard = Keyboard::new(RecordingKeyboardDispatcher);
        let mut mouse = Mouse::new(dispatcher, &keyboard);
        mouse.click(7.0, 9.0, MouseButton::Left).unwrap();
        assert_eq!(
            *events.lock().unwrap(),
            vec![
                mouse_payload(
                    MouseEventType::Moved,
                    7.0,
                    9.0,
                    Some(MouseButton::None),
                    Some(0),
                    None,
                    Some(0.0),
                ),
                mouse_payload(
                    MouseEventType::Pressed,
                    7.0,
                    9.0,
                    Some(MouseButton::Left),
                    Some(1),
                    Some(1),
                    Some(0.5),
                ),
                mouse_payload(
                    MouseEventType::Released,
                    7.0,
                    9.0,
                    Some(MouseButton::Left),
                    Some(0),
                    Some(1),
                    None,
                ),
            ]
        );
    }

    #[test]
    fn dblclick_uses_click_counts_one_then_two() {
        let dispatcher = RecordingMouseDispatcher::default();
        let events = dispatcher.events.clone();
        let keyboard = Keyboard::new(RecordingKeyboardDispatcher);
        let mut mouse = Mouse::new(dispatcher, &keyboard);
        mouse.dblclick(7.0, 9.0, MouseButton::Left).unwrap();
        assert_eq!(
            *events.lock().unwrap(),
            vec![
                mouse_payload(
                    MouseEventType::Moved,
                    7.0,
                    9.0,
                    Some(MouseButton::None),
                    Some(0),
                    None,
                    Some(0.0),
                ),
                mouse_payload(
                    MouseEventType::Pressed,
                    7.0,
                    9.0,
                    Some(MouseButton::Left),
                    Some(1),
                    Some(1),
                    Some(0.5),
                ),
                mouse_payload(
                    MouseEventType::Released,
                    7.0,
                    9.0,
                    Some(MouseButton::Left),
                    Some(0),
                    Some(1),
                    None,
                ),
                mouse_payload(
                    MouseEventType::Pressed,
                    7.0,
                    9.0,
                    Some(MouseButton::Left),
                    Some(1),
                    Some(2),
                    Some(0.5),
                ),
                mouse_payload(
                    MouseEventType::Released,
                    7.0,
                    9.0,
                    Some(MouseButton::Left),
                    Some(0),
                    Some(2),
                    None,
                ),
            ]
        );
    }

    #[test]
    fn move_uses_half_force_while_button_is_held() {
        let dispatcher = RecordingMouseDispatcher::default();
        let events = dispatcher.events.clone();
        let keyboard = Keyboard::new(RecordingKeyboardDispatcher);
        let mut mouse = Mouse::new(dispatcher, &keyboard);
        mouse.down(MouseButton::Right, 1).unwrap();
        events.lock().unwrap().clear();
        mouse.move_to(2.0, 3.0, 1).unwrap();
        assert_eq!(
            *events.lock().unwrap(),
            vec![mouse_payload(
                MouseEventType::Moved,
                2.0,
                3.0,
                Some(MouseButton::Right),
                Some(2),
                None,
                Some(0.5),
            )]
        );
    }

    #[test]
    fn wheel_emits_current_position_and_deltas() {
        let dispatcher = RecordingMouseDispatcher::default();
        let events = dispatcher.events.clone();
        let keyboard = Keyboard::new(RecordingKeyboardDispatcher);
        let mut mouse = Mouse::new(dispatcher, &keyboard);
        mouse.hover(3.0, 4.0).unwrap();
        events.lock().unwrap().clear();
        mouse.wheel(-12.5, 80.0).unwrap();
        assert_eq!(
            *events.lock().unwrap(),
            vec![MouseDispatch::Mouse(MouseEventPayload {
                event_type: MouseEventType::Wheel,
                x: 3.0,
                y: 4.0,
                button: None,
                buttons: None,
                modifiers: 0,
                click_count: None,
                force: None,
                delta_x: Some(-12.5),
                delta_y: Some(80.0),
            })]
        );
    }

    #[test]
    fn tap_emits_touch_start_then_empty_touch_end() {
        let dispatcher = RecordingMouseDispatcher::default();
        let events = dispatcher.events.clone();
        let keyboard = Keyboard::new(RecordingKeyboardDispatcher);
        let mut mouse = Mouse::new(dispatcher, &keyboard);
        mouse.tap(5.0, 6.0).unwrap();
        assert_eq!(
            *events.lock().unwrap(),
            vec![
                MouseDispatch::Touch(TouchEventPayload {
                    event_type: TouchEventType::Start,
                    touch_points: vec![MainFrameCssPoint { x: 5.0, y: 6.0 }],
                    modifiers: 0,
                }),
                MouseDispatch::Touch(TouchEventPayload {
                    event_type: TouchEventType::End,
                    touch_points: Vec::new(),
                    modifiers: 0,
                }),
            ]
        );
    }

    #[test]
    fn mouse_reads_live_keyboard_modifiers() {
        let dispatcher = RecordingMouseDispatcher::default();
        let events = dispatcher.events.clone();
        let mut keyboard = Keyboard::new(RecordingKeyboardDispatcher);
        keyboard.down("Control").unwrap();
        let mut mouse = Mouse::new(dispatcher, &keyboard);
        mouse.hover(1.0, 1.0).unwrap();
        let MouseDispatch::Mouse(payload) = &events.lock().unwrap()[0] else {
            panic!("expected mouse event");
        };
        assert_eq!(payload.modifiers, 2);
    }

    #[test]
    fn scroll_strategy_cycles_in_playwright_order() {
        let mut strategy = ScrollStrategy::Protocol;
        let mut observed = Vec::new();
        for attempt in 0..8 {
            observed.push((ScrollStrategy::for_attempt(attempt), strategy));
            strategy = strategy.advance();
        }
        assert_eq!(
            observed,
            vec![
                (ScrollStrategy::Protocol, ScrollStrategy::Protocol),
                (ScrollStrategy::End, ScrollStrategy::End),
                (ScrollStrategy::Center, ScrollStrategy::Center),
                (ScrollStrategy::Start, ScrollStrategy::Start),
                (ScrollStrategy::Protocol, ScrollStrategy::Protocol),
                (ScrollStrategy::End, ScrollStrategy::End),
                (ScrollStrategy::Center, ScrollStrategy::Center),
                (ScrollStrategy::Start, ScrollStrategy::Start),
            ]
        );
    }
}
