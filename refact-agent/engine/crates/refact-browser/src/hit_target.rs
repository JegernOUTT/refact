use std::collections::HashMap;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use headless_chrome::Tab;
use headless_chrome::protocol::cdp::{DOM, Page, Runtime};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{ActionKind, ElementHandle, HandleError, WorldManager};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct HitTargetPoint {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HitTargetResult {
    Done,
    Intercepted { description: String },
    NotConnected,
    Skipped,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub struct InterceptorToken(u64);

#[derive(Clone, Debug, PartialEq)]
pub struct FrameOwnerGeometry {
    pub x: f64,
    pub y: f64,
    pub border_left: f64,
    pub border_top: f64,
    pub transformed: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FramePointTranslation {
    Supported(HitTargetPoint),
    Unsupported { reason: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HitTargetError {
    Protocol(String),
    InvalidResult(String),
    UnknownToken(InterceptorToken),
}

impl Display for HitTargetError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Protocol(message) | Self::InvalidResult(message) => formatter.write_str(message),
            Self::UnknownToken(token) => {
                write!(formatter, "Unknown hit-target interceptor {}", token.0)
            }
        }
    }
}

impl Error for HitTargetError {}

impl From<HandleError> for HitTargetError {
    fn from(error: HandleError) -> Self {
        Self::Protocol(error.to_string())
    }
}

pub trait FrameHitTargetDriver {
    fn parent_frame(&mut self, frame_id: &str) -> Result<Option<String>, HitTargetError>;

    fn owner_geometry(
        &mut self,
        child_frame_id: &str,
    ) -> Result<FramePointTranslationGeometry, HitTargetError>;

    fn expect_owner_hit(
        &mut self,
        child_frame_id: &str,
        point_in_parent: HitTargetPoint,
    ) -> Result<HitTargetResult, HitTargetError>;
}

pub struct CdpFrameHitTargetDriver<'a> {
    tab: &'a Tab,
    frame_tree: Page::FrameTree,
    owner_objects: HashMap<String, String>,
}

impl<'a> CdpFrameHitTargetDriver<'a> {
    pub fn new(tab: &'a Tab, worlds: &WorldManager) -> Result<Self, HitTargetError> {
        worlds
            .ensure_utility_world(tab)
            .map_err(HitTargetError::Protocol)?;
        let frame_tree = tab
            .call_method(Page::GetFrameTree(None))
            .map_err(|error| {
                HitTargetError::Protocol(format!("Failed to read browser frame tree: {error}"))
            })?
            .frame_tree;
        Ok(Self {
            tab,
            frame_tree,
            owner_objects: HashMap::new(),
        })
    }

    fn frame(&self, frame_id: &str) -> Option<&Page::Frame> {
        find_frame(&self.frame_tree, frame_id)
    }

    fn resolve_owner_object(
        &mut self,
        child_frame_id: &str,
    ) -> Result<String, FramePointTranslationGeometry> {
        if let Some(object_id) = self.owner_objects.get(child_frame_id) {
            return Ok(object_id.clone());
        }
        let parent_frame_id = self
            .frame(child_frame_id)
            .and_then(|frame| frame.parent_id.clone())
            .ok_or_else(|| FramePointTranslationGeometry::Unsupported {
                reason: format!("frame {child_frame_id} has no parent"),
            })?;
        let context_id = self
            .tab
            .call_method(Page::CreateIsolatedWorld {
                frame_id: parent_frame_id,
                world_name: Some(crate::UTILITY_WORLD_NAME.to_string()),
                grant_univeral_access: Some(true),
            })
            .map_err(|error| FramePointTranslationGeometry::Unsupported {
                reason: format!(
                    "cannot access parent utility world for frame {child_frame_id}: {error}"
                ),
            })?
            .execution_context_id;
        let owner = self
            .tab
            .call_method(DOM::GetFrameOwner {
                frame_id: child_frame_id.to_string(),
            })
            .map_err(|error| FramePointTranslationGeometry::Unsupported {
                reason: format!(
                    "out-of-process iframe owner for {child_frame_id} is unsupported until T-27: {error}"
                ),
            })?;
        let object = self
            .tab
            .call_method(DOM::ResolveNode {
                node_id: None,
                backend_node_id: Some(owner.backend_node_id),
                object_group: None,
                execution_context_id: Some(context_id),
            })
            .map_err(|error| FramePointTranslationGeometry::Unsupported {
                reason: format!("cannot resolve iframe owner for {child_frame_id}: {error}"),
            })?
            .object;
        let object_id =
            object
                .object_id
                .ok_or_else(|| FramePointTranslationGeometry::Unsupported {
                    reason: format!("iframe owner for {child_frame_id} has no remote object"),
                })?;
        self.owner_objects
            .insert(child_frame_id.to_string(), object_id.clone());
        Ok(object_id)
    }
}

impl FrameHitTargetDriver for CdpFrameHitTargetDriver<'_> {
    fn parent_frame(&mut self, frame_id: &str) -> Result<Option<String>, HitTargetError> {
        self.frame(frame_id)
            .map(|frame| frame.parent_id.clone())
            .ok_or_else(|| HitTargetError::Protocol(format!("Unknown browser frame {frame_id}")))
    }

    fn owner_geometry(
        &mut self,
        child_frame_id: &str,
    ) -> Result<FramePointTranslationGeometry, HitTargetError> {
        let object_id = match self.resolve_owner_object(child_frame_id) {
            Ok(object_id) => object_id,
            Err(unsupported) => return Ok(unsupported),
        };
        let model = self
            .tab
            .call_method(DOM::GetBoxModel {
                node_id: None,
                backend_node_id: None,
                object_id: Some(object_id.clone()),
            })
            .map_err(|error| {
                HitTargetError::Protocol(format!(
                    "Failed to read iframe owner box for {child_frame_id}: {error}"
                ))
            })?
            .model;
        let style = self.call_owner_function(
            &object_id,
            "function() { if (!this.ownerDocument || !this.ownerDocument.defaultView) return { status: 'not_connected' }; const view = this.ownerDocument.defaultView; for (let element = this; element; element = element.parentElement || (element.parentNode && element.parentNode.host)) { if (view.getComputedStyle(element).transform !== 'none') return { status: 'transformed' }; } const style = view.getComputedStyle(this); return { status: 'done', left: parseInt(style.borderLeftWidth || '', 10) + parseInt(style.paddingLeft || '', 10), top: parseInt(style.borderTopWidth || '', 10) + parseInt(style.paddingTop || '', 10) }; }",
            Vec::new(),
        )?;
        match style.get("status").and_then(Value::as_str) {
            Some("transformed") => Ok(FramePointTranslationGeometry::Supported(
                FrameOwnerGeometry {
                    x: model.border.first().copied().unwrap_or(0.0),
                    y: model.border.get(1).copied().unwrap_or(0.0),
                    border_left: 0.0,
                    border_top: 0.0,
                    transformed: true,
                },
            )),
            Some("not_connected") => Ok(FramePointTranslationGeometry::Unsupported {
                reason: format!("iframe owner for {child_frame_id} is not connected"),
            }),
            Some("done") => Ok(FramePointTranslationGeometry::Supported(
                FrameOwnerGeometry {
                    x: model.border.first().copied().unwrap_or(0.0),
                    y: model.border.get(1).copied().unwrap_or(0.0),
                    border_left: style.get("left").and_then(Value::as_f64).unwrap_or(0.0),
                    border_top: style.get("top").and_then(Value::as_f64).unwrap_or(0.0),
                    transformed: false,
                },
            )),
            _ => Err(HitTargetError::InvalidResult(format!(
                "Unknown iframe owner style result: {style}"
            ))),
        }
    }

    fn expect_owner_hit(
        &mut self,
        child_frame_id: &str,
        point_in_parent: HitTargetPoint,
    ) -> Result<HitTargetResult, HitTargetError> {
        let object_id = match self.resolve_owner_object(child_frame_id) {
            Ok(object_id) => object_id,
            Err(FramePointTranslationGeometry::Unsupported { reason }) => {
                return Ok(HitTargetResult::Intercepted {
                    description: reason,
                });
            }
            Err(FramePointTranslationGeometry::Supported(_)) => unreachable!(),
        };
        let value = self.call_owner_function(
            &object_id,
            "function(point) { const instance = globalThis.__refact_injected__; if (!instance) throw new Error('RefactInjected is not installed'); return instance.expectHitTarget(this, point); }",
            vec![json!(point_in_parent)],
        )?;
        parse_result(&value)
    }
}

impl CdpFrameHitTargetDriver<'_> {
    fn call_owner_function(
        &self,
        object_id: &str,
        function_declaration: &str,
        arguments: Vec<Value>,
    ) -> Result<Value, HitTargetError> {
        let result = self
            .tab
            .call_method(Runtime::CallFunctionOn {
                function_declaration: function_declaration.to_string(),
                object_id: Some(object_id.to_string()),
                arguments: Some(
                    arguments
                        .into_iter()
                        .map(|value| Runtime::CallArgument {
                            value: Some(value),
                            unserializable_value: None,
                            object_id: None,
                        })
                        .collect(),
                ),
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
                HitTargetError::Protocol(format!("Failed to call browser iframe owner: {error}"))
            })?;
        if let Some(exception) = result.exception_details {
            return Err(HitTargetError::Protocol(
                exception
                    .exception
                    .and_then(|exception| exception.description)
                    .unwrap_or(exception.text),
            ));
        }
        Ok(result.result.value.unwrap_or(Value::Null))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum FramePointTranslationGeometry {
    Supported(FrameOwnerGeometry),
    Unsupported { reason: String },
}

#[derive(Clone)]
struct InterceptorEntry {
    handle: ElementHandle,
    remote_id: Option<u64>,
    immediate: Option<HitTargetResult>,
}

#[derive(Clone, Default)]
pub struct HitTargetController {
    entries: Arc<Mutex<HashMap<InterceptorToken, InterceptorEntry>>>,
    next_token: Arc<AtomicU64>,
}

impl HitTargetController {
    pub fn expect_hit_target(
        &self,
        tab: &Tab,
        worlds: &WorldManager,
        handle: &ElementHandle,
        point: HitTargetPoint,
    ) -> Result<HitTargetResult, HitTargetError> {
        let value = worlds.call_function_on(
            tab,
            handle,
            "function(point) { const instance = globalThis.__refact_injected__; if (!instance) throw new Error('RefactInjected is not installed'); return instance.expectHitTarget(this, point); }",
            vec![json!(point)],
        )?;
        parse_result(&value)
    }

    pub fn install_interceptor(
        &self,
        tab: &Tab,
        worlds: &WorldManager,
        handle: &ElementHandle,
        action_kind: ActionKind,
        point: Option<HitTargetPoint>,
    ) -> Result<InterceptorToken, HitTargetError> {
        if matches!(action_kind, ActionKind::DragSource | ActionKind::DragTarget) {
            return Ok(self.insert_entry(handle.clone(), None, Some(HitTargetResult::Skipped)));
        }
        let Some(action) = interceptor_action(action_kind) else {
            return Ok(self.insert_entry(handle.clone(), None, Some(HitTargetResult::Skipped)));
        };
        let value = worlds.call_function_on(
            tab,
            handle,
            "function(action, point) { const instance = globalThis.__refact_injected__; if (!instance) throw new Error('RefactInjected is not installed'); return instance.installHitTargetInterceptor(this, action, point === null ? undefined : point); }",
            vec![json!(action), point.map_or(Value::Null, |point| json!(point))],
        )?;
        if value.get("status").and_then(Value::as_str) == Some("installed") {
            let remote_id = value.get("id").and_then(Value::as_u64).ok_or_else(|| {
                HitTargetError::InvalidResult(
                    "Hit-target interceptor install returned no numeric id".to_string(),
                )
            })?;
            return Ok(self.insert_entry(handle.clone(), Some(remote_id), None));
        }
        let result = parse_result(&value)?;
        Ok(self.insert_entry(handle.clone(), None, Some(result)))
    }

    pub fn skip_interceptor(&self, handle: &ElementHandle) -> InterceptorToken {
        self.insert_entry(handle.clone(), None, Some(HitTargetResult::Skipped))
    }

    pub fn take_result(
        &self,
        tab: &Tab,
        worlds: &WorldManager,
        token: InterceptorToken,
    ) -> Result<HitTargetResult, HitTargetError> {
        let entry = self
            .entries
            .lock()
            .unwrap()
            .remove(&token)
            .ok_or(HitTargetError::UnknownToken(token))?;
        if let Some(result) = entry.immediate {
            return Ok(result);
        }
        let remote_id = entry.remote_id.ok_or_else(|| {
            HitTargetError::InvalidResult("Hit-target interceptor has no result source".to_string())
        })?;
        let value = worlds.call_function_on(
            tab,
            &entry.handle,
            "function(id) { const instance = globalThis.__refact_injected__; if (!instance) throw new Error('RefactInjected is not installed'); return instance.takeHitTargetInterceptor(id); }",
            vec![json!(remote_id)],
        )?;
        parse_result(&value)
    }

    fn insert_entry(
        &self,
        handle: ElementHandle,
        remote_id: Option<u64>,
        immediate: Option<HitTargetResult>,
    ) -> InterceptorToken {
        let token = InterceptorToken(self.next_token.fetch_add(1, Ordering::Relaxed) + 1);
        self.entries.lock().unwrap().insert(
            token,
            InterceptorEntry {
                handle,
                remote_id,
                immediate,
            },
        );
        token
    }

    #[cfg(test)]
    fn take_immediate_result(
        &self,
        token: InterceptorToken,
    ) -> Result<HitTargetResult, HitTargetError> {
        let entry = self
            .entries
            .lock()
            .unwrap()
            .remove(&token)
            .ok_or(HitTargetError::UnknownToken(token))?;
        entry.immediate.ok_or_else(|| {
            HitTargetError::InvalidResult("Hit-target interceptor result is remote".to_string())
        })
    }
}

pub fn install_interceptor(
    controller: &HitTargetController,
    tab: &Tab,
    worlds: &WorldManager,
    handle: &ElementHandle,
    action_kind: ActionKind,
    point: Option<HitTargetPoint>,
) -> Result<InterceptorToken, HitTargetError> {
    controller.install_interceptor(tab, worlds, handle, action_kind, point)
}

pub fn take_result(
    controller: &HitTargetController,
    tab: &Tab,
    worlds: &WorldManager,
    token: InterceptorToken,
) -> Result<HitTargetResult, HitTargetError> {
    controller.take_result(tab, worlds, token)
}

pub fn translate_point_to_frame(
    driver: &mut impl FrameHitTargetDriver,
    target_frame_id: &str,
    main_frame_point: HitTargetPoint,
) -> Result<FramePointTranslation, HitTargetError> {
    let mut child_frames = Vec::new();
    let mut frame_id = target_frame_id.to_string();
    while let Some(parent_frame_id) = driver.parent_frame(&frame_id)? {
        child_frames.push(frame_id);
        frame_id = parent_frame_id;
    }
    child_frames.reverse();

    let mut point = main_frame_point;
    for child_frame_id in child_frames {
        let geometry = match driver.owner_geometry(&child_frame_id)? {
            FramePointTranslationGeometry::Supported(geometry) => geometry,
            FramePointTranslationGeometry::Unsupported { reason } => {
                return Ok(FramePointTranslation::Unsupported { reason });
            }
        };
        if geometry.transformed {
            return Ok(FramePointTranslation::Unsupported {
                reason: format!("iframe owner for {child_frame_id} is transformed"),
            });
        }
        match driver.expect_owner_hit(&child_frame_id, point)? {
            HitTargetResult::Done => {}
            HitTargetResult::Intercepted { description } => {
                return Ok(FramePointTranslation::Unsupported {
                    reason: description,
                });
            }
            HitTargetResult::NotConnected => {
                return Ok(FramePointTranslation::Unsupported {
                    reason: format!("iframe owner for {child_frame_id} is not connected"),
                });
            }
            HitTargetResult::Skipped => {
                return Ok(FramePointTranslation::Unsupported {
                    reason: format!("iframe owner hit test for {child_frame_id} was skipped"),
                });
            }
        }
        point = HitTargetPoint {
            x: point.x - geometry.x - geometry.border_left,
            y: point.y - geometry.y - geometry.border_top,
        };
    }
    Ok(FramePointTranslation::Supported(point))
}

pub fn translate_point_to_frame_cdp(
    tab: &Tab,
    worlds: &WorldManager,
    target_frame_id: &str,
    main_frame_point: HitTargetPoint,
) -> Result<FramePointTranslation, HitTargetError> {
    let mut driver = CdpFrameHitTargetDriver::new(tab, worlds)?;
    translate_point_to_frame(&mut driver, target_frame_id, main_frame_point)
}

fn find_frame<'a>(frame_tree: &'a Page::FrameTree, frame_id: &str) -> Option<&'a Page::Frame> {
    if frame_tree.frame.id == frame_id {
        return Some(&frame_tree.frame);
    }
    frame_tree
        .child_frames
        .as_ref()?
        .iter()
        .find_map(|child| find_frame(child, frame_id))
}

fn interceptor_action(action_kind: ActionKind) -> Option<&'static str> {
    match action_kind {
        ActionKind::Click | ActionKind::DblClick | ActionKind::Check | ActionKind::Uncheck => {
            Some("mouse")
        }
        ActionKind::Hover => Some("hover"),
        ActionKind::Tap => Some("tap"),
        ActionKind::DragSource | ActionKind::DragTarget => Some("drag"),
        ActionKind::Fill
        | ActionKind::Type
        | ActionKind::Press
        | ActionKind::SelectOption
        | ActionKind::SetInputFiles
        | ActionKind::Focus
        | ActionKind::ScrollIntoViewIfNeeded => None,
    }
}

fn parse_result(value: &Value) -> Result<HitTargetResult, HitTargetError> {
    match value.get("status").and_then(Value::as_str) {
        Some("done") => Ok(HitTargetResult::Done),
        Some("not_connected") => Ok(HitTargetResult::NotConnected),
        Some("skipped") => Ok(HitTargetResult::Skipped),
        Some("intercepted") => value
            .get("description")
            .and_then(Value::as_str)
            .map(|description| HitTargetResult::Intercepted {
                description: description.to_string(),
            })
            .ok_or_else(|| {
                HitTargetError::InvalidResult(
                    "Hit-target intercepted result has no description".to_string(),
                )
            }),
        _ => Err(HitTargetError::InvalidResult(format!(
            "Unknown hit-target result: {value}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct RecordingFrameDriver {
        parents: HashMap<String, String>,
        geometry: HashMap<String, FramePointTranslationGeometry>,
        hits: Vec<(String, HitTargetPoint)>,
        hit_result: Option<HitTargetResult>,
    }

    impl FrameHitTargetDriver for RecordingFrameDriver {
        fn parent_frame(&mut self, frame_id: &str) -> Result<Option<String>, HitTargetError> {
            Ok(self.parents.get(frame_id).cloned())
        }

        fn owner_geometry(
            &mut self,
            child_frame_id: &str,
        ) -> Result<FramePointTranslationGeometry, HitTargetError> {
            Ok(self.geometry.get(child_frame_id).cloned().unwrap())
        }

        fn expect_owner_hit(
            &mut self,
            child_frame_id: &str,
            point_in_parent: HitTargetPoint,
        ) -> Result<HitTargetResult, HitTargetError> {
            self.hits
                .push((child_frame_id.to_string(), point_in_parent));
            Ok(self.hit_result.clone().unwrap_or(HitTargetResult::Done))
        }
    }

    fn handle() -> ElementHandle {
        ElementHandle {
            object_id: "object".to_string(),
            backend_node_id: Some(1),
            context_id: 2,
            frame_id: "main".to_string(),
        }
    }

    #[test]
    fn token_state_machine_consumes_immediate_result_once() {
        let controller = HitTargetController::default();
        let token = controller.skip_interceptor(&handle());
        assert_eq!(
            controller.take_immediate_result(token).unwrap(),
            HitTargetResult::Skipped
        );
        assert_eq!(
            controller.take_immediate_result(token),
            Err(HitTargetError::UnknownToken(token))
        );
    }

    #[test]
    fn tokens_are_unique_and_unknown_tokens_are_rejected() {
        let controller = HitTargetController::default();
        let first = controller.insert_entry(handle(), None, Some(HitTargetResult::Done));
        let second = controller.insert_entry(handle(), None, Some(HitTargetResult::Done));
        assert_ne!(first, second);
        let missing = InterceptorToken(999);
        assert!(matches!(
            controller.entries.lock().unwrap().remove(&missing),
            None
        ));
    }

    #[test]
    fn drag_actions_are_classified_for_skip_before_installation() {
        assert_eq!(interceptor_action(ActionKind::DragSource), Some("drag"));
        assert_eq!(interceptor_action(ActionKind::DragTarget), Some("drag"));
        assert_eq!(interceptor_action(ActionKind::Click), Some("mouse"));
    }

    #[test]
    fn frame_translation_checks_each_owner_and_accumulates_offsets() {
        let mut driver = RecordingFrameDriver::default();
        driver
            .parents
            .insert("inner".to_string(), "outer".to_string());
        driver
            .parents
            .insert("outer".to_string(), "main".to_string());
        driver.geometry.insert(
            "outer".to_string(),
            FramePointTranslationGeometry::Supported(FrameOwnerGeometry {
                x: 20.0,
                y: 30.0,
                border_left: 2.0,
                border_top: 3.0,
                transformed: false,
            }),
        );
        driver.geometry.insert(
            "inner".to_string(),
            FramePointTranslationGeometry::Supported(FrameOwnerGeometry {
                x: 5.0,
                y: 7.0,
                border_left: 1.0,
                border_top: 1.0,
                transformed: false,
            }),
        );
        assert_eq!(
            translate_point_to_frame(&mut driver, "inner", HitTargetPoint { x: 100.0, y: 90.0 })
                .unwrap(),
            FramePointTranslation::Supported(HitTargetPoint { x: 72.0, y: 49.0 })
        );
        assert_eq!(
            driver.hits,
            vec![
                ("outer".to_string(), HitTargetPoint { x: 100.0, y: 90.0 }),
                ("inner".to_string(), HitTargetPoint { x: 78.0, y: 57.0 }),
            ]
        );
    }

    #[test]
    fn oopif_and_transformed_owners_are_unsupported() {
        let mut oopif = RecordingFrameDriver::default();
        oopif
            .parents
            .insert("child".to_string(), "main".to_string());
        oopif.geometry.insert(
            "child".to_string(),
            FramePointTranslationGeometry::Unsupported {
                reason: "out-of-process iframe until T-27".to_string(),
            },
        );
        assert_eq!(
            translate_point_to_frame(&mut oopif, "child", HitTargetPoint { x: 10.0, y: 20.0 })
                .unwrap(),
            FramePointTranslation::Unsupported {
                reason: "out-of-process iframe until T-27".to_string()
            }
        );

        let mut transformed = RecordingFrameDriver::default();
        transformed
            .parents
            .insert("child".to_string(), "main".to_string());
        transformed.geometry.insert(
            "child".to_string(),
            FramePointTranslationGeometry::Supported(FrameOwnerGeometry {
                x: 0.0,
                y: 0.0,
                border_left: 0.0,
                border_top: 0.0,
                transformed: true,
            }),
        );
        assert!(matches!(
            translate_point_to_frame(
                &mut transformed,
                "child",
                HitTargetPoint { x: 10.0, y: 20.0 }
            )
            .unwrap(),
            FramePointTranslation::Unsupported { .. }
        ));
    }
}
