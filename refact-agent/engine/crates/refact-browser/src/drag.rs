use std::path::Path;
use std::sync::{Arc, Weak, mpsc};
use std::time::Duration;

use base64::Engine;
use headless_chrome::Tab;
use headless_chrome::browser::tab::EventListener;
use headless_chrome::protocol::cdp::types::Event;
use headless_chrome::protocol::cdp::Input;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    ElementHandle, KeyboardDispatcher, MainFrameCssPoint, Mouse, MouseButton, MouseDispatcher,
    MouseError, WorldManager,
};

const DRAG_DATA_TIMEOUT: Duration = Duration::from_secs(5);
const DRAG_START_DISTANCE: f64 = 10.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragEventType {
    Enter,
    Over,
    Drop,
    Cancel,
}

pub trait DragDispatcher {
    fn begin_interception(&mut self, source_frame_id: &str) -> Result<(), String>;
    fn finish_interception(&mut self) -> Result<Input::DragData, String>;
    fn dispatch_drag(
        &mut self,
        event_type: DragEventType,
        point: MainFrameCssPoint,
        data: &Input::DragData,
        modifiers: u32,
    ) -> Result<(), String>;
}

type DragListener = dyn EventListener<Event> + Send + Sync;

pub struct CdpDragDispatcher<'a> {
    tab: &'a Tab,
    world: &'a WorldManager,
    source_frame_id: Option<String>,
    receiver: Option<mpsc::Receiver<Input::DragData>>,
    listener: Option<Weak<DragListener>>,
}

impl<'a> CdpDragDispatcher<'a> {
    pub fn new(tab: &'a Tab, world: &'a WorldManager) -> Self {
        Self {
            tab,
            world,
            source_frame_id: None,
            receiver: None,
            listener: None,
        }
    }

    fn stop_interception(&mut self) -> Result<(), String> {
        let result = self
            .tab
            .call_method(Input::SetInterceptDrags { enabled: false })
            .map(|_| ())
            .map_err(|error| format!("Failed to disable browser drag interception: {error}"));
        if let Some(listener) = self.listener.take() {
            let _ = self.tab.remove_event_listener(&listener);
        }
        result
    }
}

impl DragDispatcher for CdpDragDispatcher<'_> {
    fn begin_interception(&mut self, source_frame_id: &str) -> Result<(), String> {
        let setup = r#"(() => {
  let dragEvent = null;
  let didStartDrag = Promise.resolve(false);
  const dragListener = event => { dragEvent = event; };
  const mouseListener = () => {
    didStartDrag = new Promise(resolve => {
      window.addEventListener('dragstart', dragListener, {once:true,capture:true});
      setTimeout(() => resolve(dragEvent ? !dragEvent.defaultPrevented : false), 0);
    });
  };
  window.addEventListener('mousemove', mouseListener, {once:true,capture:true});
  window.__refactCleanupDrag = async () => {
    const result = await didStartDrag;
    window.removeEventListener('mousemove', mouseListener, {capture:true});
    window.removeEventListener('dragstart', dragListener, {capture:true});
    delete window.__refactCleanupDrag;
    return result;
  };
})()"#;
        self.world
            .eval_in_utility_frame(self.tab, source_frame_id, setup)?;
        let (sender, receiver) = mpsc::sync_channel(1);
        let listener: Arc<DragListener> = Arc::new(move |event: &Event| {
            if let Event::InputDragIntercepted(event) = event {
                let _ = sender.try_send(event.params.data.clone());
            }
        });
        self.listener = Some(self.tab.add_event_listener(listener).map_err(|error| {
            format!("Failed to listen for intercepted browser drag data: {error}")
        })?);
        self.receiver = Some(receiver);
        self.source_frame_id = Some(source_frame_id.to_string());
        self.tab
            .call_method(Input::SetInterceptDrags { enabled: true })
            .map(|_| ())
            .map_err(|error| format!("Failed to enable browser drag interception: {error}"))
    }

    fn finish_interception(&mut self) -> Result<Input::DragData, String> {
        let source_frame_id = self
            .source_frame_id
            .take()
            .ok_or_else(|| "Browser drag interception was not started".to_string())?;
        let expecting_drag = self
            .world
            .eval_in_utility_frame(
                self.tab,
                &source_frame_id,
                "window.__refactCleanupDrag?.() ?? false",
            )
            .and_then(|value| {
                value
                    .as_bool()
                    .ok_or_else(|| "Browser drag listener returned an invalid result".to_string())
            });
        let received = self
            .receiver
            .take()
            .ok_or_else(|| "Browser drag interception receiver is unavailable".to_string())?
            .recv_timeout(DRAG_DATA_TIMEOUT)
            .map_err(|_| "Timed out waiting for intercepted browser drag data".to_string());
        let stop_result = self.stop_interception();
        let expecting_drag = expecting_drag?;
        stop_result?;
        if !expecting_drag {
            return Err("Element did not start an HTML5 drag".to_string());
        }
        received
    }

    fn dispatch_drag(
        &mut self,
        event_type: DragEventType,
        point: MainFrameCssPoint,
        data: &Input::DragData,
        modifiers: u32,
    ) -> Result<(), String> {
        self.tab
            .call_method(Input::DispatchDragEvent {
                Type: match event_type {
                    DragEventType::Enter => Input::DispatchDragEventTypeOption::DragEnter,
                    DragEventType::Over => Input::DispatchDragEventTypeOption::DragOver,
                    DragEventType::Drop => Input::DispatchDragEventTypeOption::Drop,
                    DragEventType::Cancel => Input::DispatchDragEventTypeOption::DragCancel,
                },
                x: point.x,
                y: point.y,
                data: data.clone(),
                modifiers: Some(modifiers),
            })
            .map(|_| ())
            .map_err(|error| format!("Failed to dispatch browser drag event: {error}"))
    }
}

impl Drop for CdpDragDispatcher<'_> {
    fn drop(&mut self) {
        if self.source_frame_id.is_some() {
            let _ = self.stop_interception();
        }
    }
}

pub fn drag_and_drop<D, K, P>(
    mouse: &mut Mouse<'_, D, K>,
    protocol: &mut P,
    source_frame_id: &str,
    source: MainFrameCssPoint,
    target: MainFrameCssPoint,
    steps: usize,
) -> Result<(), MouseError>
where
    D: MouseDispatcher,
    K: KeyboardDispatcher,
    P: DragDispatcher,
{
    mouse.move_to(source.x, source.y, 1)?;
    mouse.down(MouseButton::Left, 1)?;
    protocol
        .begin_interception(source_frame_id)
        .map_err(MouseError::Protocol)?;
    let trigger = MainFrameCssPoint {
        x: source.x + DRAG_START_DISTANCE,
        y: source.y,
    };
    mouse.move_to(trigger.x, trigger.y, 1)?;
    let data = protocol
        .finish_interception()
        .map_err(MouseError::Protocol)?;
    let modifiers = 0;
    protocol
        .dispatch_drag(DragEventType::Enter, trigger, &data, modifiers)
        .map_err(MouseError::Protocol)?;
    let steps = steps.max(2);
    let result = (|| {
        for step in 1..=steps {
            let progress = step as f64 / steps as f64;
            let point = MainFrameCssPoint {
                x: trigger.x + (target.x - trigger.x) * progress,
                y: trigger.y + (target.y - trigger.y) * progress,
            };
            protocol.dispatch_drag(DragEventType::Over, point, &data, modifiers)?;
            mouse.set_position(point);
        }
        protocol.dispatch_drag(DragEventType::Drop, target, &data, modifiers)
    })();
    if let Err(error) = result {
        let _ = protocol.dispatch_drag(DragEventType::Cancel, target, &data, modifiers);
        mouse.reset_buttons();
        return Err(MouseError::Protocol(error));
    }
    mouse.reset_buttons();
    Ok(())
}

#[derive(Serialize)]
struct FilePayload {
    name: String,
    data: String,
    last_modified: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct FileDropTarget {
    pub id: String,
    pub tag: String,
    pub connected: bool,
    pub same_document: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct FileDropResult {
    pub accepted: bool,
    pub delivered: bool,
    pub observed_names: Vec<String>,
    pub mutations: usize,
    pub target: FileDropTarget,
}

const DROP_FILES_JS: &str = r#"function(payloads) {
  const transfer = new DataTransfer();
  for (const payload of payloads) {
    const binary = atob(payload.data);
    const bytes = Uint8Array.from(binary, char => char.charCodeAt(0));
    transfer.items.add(new File([bytes], payload.name, {lastModified: payload.last_modified}));
  }
  const observed_names = Array.from(transfer.files).map(file => file.name);
  const target = {
    id: this.id || '',
    tag: this.tagName || '',
    connected: this.isConnected === true,
    same_document: this.ownerDocument === document,
  };
  const dispatch = type => {
    const event = new DragEvent(type, {bubbles:true,cancelable:true,dataTransfer:transfer});
    this.dispatchEvent(event);
    return event.defaultPrevented;
  };
  dispatch('dragenter');
  const accepted = dispatch('dragover');
  let delivered = false;
  let mutations = 0;
  if (accepted) {
    const observer = new MutationObserver(() => {});
    observer.observe(this, {attributes:true,childList:true,subtree:true,characterData:true});
    delivered = dispatch('drop');
    mutations = observer.takeRecords().length;
    observer.disconnect();
  }
  return {accepted,delivered,mutations,observed_names,target};
}"#;

pub fn verify_file_drop(result: &FileDropResult, expected: &[String]) -> Result<(), String> {
    if !result.target.connected || !result.target.same_document {
        return Err(format!(
            "Drop target <{}> is not attached to the page document",
            result.target.tag
        ));
    }
    if !result.accepted {
        return Err("Target dragover handler did not call preventDefault()".to_string());
    }
    let expected_names = expected
        .iter()
        .map(|path| {
            Path::new(path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(path)
                .to_string()
        })
        .collect::<Vec<_>>();
    if result.observed_names != expected_names {
        return Err(format!(
            "Dropped files {:?} but the page received {:?}",
            expected_names, result.observed_names
        ));
    }
    if !result.delivered && result.mutations == 0 {
        return Err(
            "Target drop handler did not run: the page produced no observable effect".to_string(),
        );
    }
    Ok(())
}

pub fn drop_files(
    tab: &Tab,
    world: &WorldManager,
    target: &ElementHandle,
    paths: &[String],
) -> Result<FileDropResult, String> {
    let mut payloads = Vec::with_capacity(paths.len());
    for value in paths {
        let path = Path::new(value)
            .canonicalize()
            .map_err(|_| format!("Upload path does not exist: {value}"))?;
        if !path.is_file() {
            return Err(format!("Drop path is not a file: {}", path.display()));
        }
        let bytes = std::fs::read(&path)
            .map_err(|error| format!("Failed to read drop file {}: {error}", path.display()))?;
        let last_modified = path
            .metadata()
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
            .unwrap_or_default();
        payloads.push(FilePayload {
            name: path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("file")
                .to_string(),
            data: base64::prelude::BASE64_STANDARD.encode(bytes),
            last_modified,
        });
    }
    let result = world
        .call_function_on(
            tab,
            target,
            DROP_FILES_JS,
            vec![serde_json::to_value(payloads).map_err(|error| error.to_string())?],
        )
        .map_err(|error| error.to_string())?;
    serde_json::from_value(result.clone())
        .map_err(|_| format!("File drop returned an invalid result: {}", json!(result)))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use crate::{Keyboard, KeyboardDispatch};

    use super::*;

    #[derive(Clone, Debug, PartialEq)]
    enum Recorded {
        Mouse(crate::MouseDispatch),
        Intercept(bool),
        Drag(DragEventType, MainFrameCssPoint),
    }

    #[derive(Clone)]
    struct RecordingDispatcher {
        events: Arc<Mutex<Vec<Recorded>>>,
        data: Input::DragData,
    }

    impl MouseDispatcher for RecordingDispatcher {
        fn dispatch(&mut self, event: crate::MouseDispatch) -> Result<(), String> {
            self.events.lock().unwrap().push(Recorded::Mouse(event));
            Ok(())
        }
    }

    impl DragDispatcher for RecordingDispatcher {
        fn begin_interception(&mut self, _source_frame_id: &str) -> Result<(), String> {
            self.events.lock().unwrap().push(Recorded::Intercept(true));
            Ok(())
        }

        fn finish_interception(&mut self) -> Result<Input::DragData, String> {
            self.events.lock().unwrap().push(Recorded::Intercept(false));
            Ok(self.data.clone())
        }

        fn dispatch_drag(
            &mut self,
            event_type: DragEventType,
            point: MainFrameCssPoint,
            _data: &Input::DragData,
            _modifiers: u32,
        ) -> Result<(), String> {
            self.events
                .lock()
                .unwrap()
                .push(Recorded::Drag(event_type, point));
            Ok(())
        }
    }

    #[derive(Default)]
    struct NoopKeyboard;

    impl KeyboardDispatcher for NoopKeyboard {
        fn dispatch(&mut self, _event: KeyboardDispatch) -> Result<(), String> {
            Ok(())
        }
    }

    fn drop_result(accepted: bool, delivered: bool, mutations: usize) -> FileDropResult {
        FileDropResult {
            accepted,
            delivered,
            observed_names: vec!["drop.txt".to_string()],
            mutations,
            target: FileDropTarget {
                id: "files".to_string(),
                tag: "DIV".to_string(),
                connected: true,
                same_document: true,
            },
        }
    }

    #[test]
    fn file_drop_verification_accepts_a_page_handler_that_claimed_the_event() {
        let paths = vec!["/tmp/drop.txt".to_string()];
        assert!(verify_file_drop(&drop_result(true, true, 0), &paths).is_ok());
        assert!(verify_file_drop(&drop_result(true, false, 1), &paths).is_ok());
    }

    #[test]
    fn file_drop_verification_rejects_a_target_that_never_handled_the_drop() {
        let error = verify_file_drop(
            &drop_result(true, false, 0),
            &vec!["/tmp/drop.txt".to_string()],
        )
        .unwrap_err();

        assert!(error.contains("drop handler did not run"));
    }

    #[test]
    fn file_drop_verification_rejects_a_target_that_refused_the_dragover() {
        let error = verify_file_drop(
            &drop_result(false, false, 0),
            &vec!["/tmp/drop.txt".to_string()],
        )
        .unwrap_err();

        assert!(error.contains("preventDefault()"));
    }

    #[test]
    fn file_drop_verification_rejects_a_detached_target() {
        let mut result = drop_result(true, true, 1);
        result.target.connected = false;

        let error = verify_file_drop(&result, &vec!["/tmp/drop.txt".to_string()]).unwrap_err();

        assert!(error.contains("not attached to the page document"));
    }

    #[test]
    fn file_drop_verification_compares_basenames_and_rejects_missing_files() {
        let mut result = drop_result(true, true, 1);
        result.observed_names.clear();

        let error = verify_file_drop(&result, &vec!["/tmp/drop.txt".to_string()]).unwrap_err();

        assert!(error.contains("[\"drop.txt\"]"));
        assert!(error.contains("[]"));
    }

    #[test]
    fn chromium_drag_sequence_intercepts_then_enters_over_twice_and_drops() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let dispatcher = RecordingDispatcher {
            events: events.clone(),
            data: Input::DragData {
                items: Vec::new(),
                files: None,
                drag_operations_mask: 1,
            },
        };
        let keyboard = Keyboard::new(NoopKeyboard);
        let mut mouse = Mouse::new(dispatcher.clone(), &keyboard);
        let mut protocol = dispatcher;

        drag_and_drop(
            &mut mouse,
            &mut protocol,
            "main",
            MainFrameCssPoint { x: 10.0, y: 20.0 },
            MainFrameCssPoint { x: 30.0, y: 40.0 },
            1,
        )
        .unwrap();

        let events = events.lock().unwrap();
        let intercept_on = events
            .iter()
            .position(|event| *event == Recorded::Intercept(true))
            .unwrap();
        let intercept_off = events
            .iter()
            .position(|event| *event == Recorded::Intercept(false))
            .unwrap();
        let drag_events = events
            .iter()
            .filter_map(|event| match event {
                Recorded::Drag(kind, point) => Some((*kind, *point)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(intercept_on < intercept_off);
        let trigger_move = events
            .iter()
            .position(|event| {
                matches!(
                    event,
                    Recorded::Mouse(crate::MouseDispatch::Mouse(crate::MouseEventPayload {
                        event_type: crate::MouseEventType::Moved,
                        x: 20.0,
                        y: 20.0,
                        button: Some(MouseButton::Left),
                        buttons: Some(1),
                        ..
                    }))
                )
            })
            .unwrap();
        let drag_enter = events
            .iter()
            .position(|event| matches!(event, Recorded::Drag(DragEventType::Enter, _)))
            .unwrap();
        assert!(intercept_on < trigger_move);
        assert!(trigger_move < intercept_off);
        assert!(intercept_off < drag_enter);
        assert_eq!(drag_events.len(), 4);
        assert_eq!(drag_events[0].0, DragEventType::Enter);
        assert_eq!(drag_events[1].0, DragEventType::Over);
        assert_eq!(drag_events[2].0, DragEventType::Over);
        assert_eq!(drag_events[3].0, DragEventType::Drop);
        assert_eq!(drag_events[3].1, MainFrameCssPoint { x: 30.0, y: 40.0 });
        assert_eq!(mouse.position(), MainFrameCssPoint { x: 30.0, y: 40.0 });
    }
}
