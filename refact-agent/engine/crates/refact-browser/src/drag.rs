use std::path::Path;
use std::time::{Duration, Instant};

use base64::Engine;
use headless_chrome::Tab;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    ElementHandle, KeyboardDispatcher, MainFrameCssPoint, Mouse, MouseButton, MouseDispatcher,
    MouseError, WorldManager,
};

const DRAG_START_DISTANCE: f64 = 10.0;
const DRAG_MOVE_STEPS: usize = 20;
const DRAG_SETTLE_TIMEOUT: Duration = Duration::from_secs(2);
const DRAG_SETTLE_INTERVAL: Duration = Duration::from_millis(25);

const DRAG_WATCH_SETUP: &str = r#"(() => {
  window.__refactDragCleanup?.();
  let started = false;
  let ended = false;
  const onDragStart = event => { started = !event.defaultPrevented; };
  const onDragEnd = () => { ended = true; };
  window.addEventListener('dragstart', onDragStart, {capture:true});
  window.addEventListener('dragend', onDragEnd, {capture:true});
  window.__refactDragWatch = () => JSON.stringify({started, ended});
  window.__refactDragCleanup = () => {
    window.removeEventListener('dragstart', onDragStart, {capture:true});
    window.removeEventListener('dragend', onDragEnd, {capture:true});
    delete window.__refactDragWatch;
    delete window.__refactDragCleanup;
  };
})()"#;

const DRAG_WATCH_POLL: &str = "window.__refactDragWatch?.() ?? \"\"";

const DRAG_WATCH_CLEANUP: &str = "window.__refactDragCleanup?.(); true";

pub trait DragObserver {
    fn watch(&mut self, source_frame_id: &str) -> Result<(), String>;
    fn settle(&mut self) -> Result<bool, String>;
}

pub struct CdpDragObserver<'a> {
    tab: &'a Tab,
    world: &'a WorldManager,
    source_frame_id: Option<String>,
}

impl<'a> CdpDragObserver<'a> {
    pub fn new(tab: &'a Tab, world: &'a WorldManager) -> Self {
        Self {
            tab,
            world,
            source_frame_id: None,
        }
    }

    fn poll(&self, source_frame_id: &str) -> Result<(bool, bool), String> {
        let value = self
            .world
            .eval_in_utility_frame(self.tab, source_frame_id, DRAG_WATCH_POLL)?;
        let state = value
            .as_str()
            .ok_or_else(|| "Browser drag observer returned an invalid result".to_string())?;
        if state.is_empty() {
            return Err("Browser drag observer is no longer installed".to_string());
        }
        let state: Value = serde_json::from_str(state)
            .map_err(|error| format!("Invalid browser drag observer state: {error}"))?;
        Ok((
            state["started"].as_bool().unwrap_or(false),
            state["ended"].as_bool().unwrap_or(false),
        ))
    }

    fn cleanup(&self, source_frame_id: &str) {
        let _ = self
            .world
            .eval_in_utility_frame(self.tab, source_frame_id, DRAG_WATCH_CLEANUP);
    }
}

impl DragObserver for CdpDragObserver<'_> {
    fn watch(&mut self, source_frame_id: &str) -> Result<(), String> {
        self.world
            .eval_in_utility_frame(self.tab, source_frame_id, DRAG_WATCH_SETUP)?;
        self.source_frame_id = Some(source_frame_id.to_string());
        Ok(())
    }

    fn settle(&mut self) -> Result<bool, String> {
        let source_frame_id = self
            .source_frame_id
            .take()
            .ok_or_else(|| "Browser drag observer was not started".to_string())?;
        let deadline = Instant::now() + DRAG_SETTLE_TIMEOUT;
        let mut started = false;
        loop {
            let (observed_start, ended) = match self.poll(&source_frame_id) {
                Ok(state) => state,
                Err(error) => {
                    self.cleanup(&source_frame_id);
                    return Err(error);
                }
            };
            started = started || observed_start;
            if ended || Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(DRAG_SETTLE_INTERVAL);
        }
        self.cleanup(&source_frame_id);
        Ok(started)
    }
}

impl Drop for CdpDragObserver<'_> {
    fn drop(&mut self) {
        if let Some(source_frame_id) = self.source_frame_id.take() {
            self.cleanup(&source_frame_id);
        }
    }
}

pub fn drag_and_drop<D, K, O>(
    mouse: &mut Mouse<'_, D, K>,
    observer: &mut O,
    source_frame_id: &str,
    source: MainFrameCssPoint,
    target: MainFrameCssPoint,
    steps: usize,
) -> Result<(), MouseError>
where
    D: MouseDispatcher,
    K: KeyboardDispatcher,
    O: DragObserver,
{
    mouse.move_to(source.x, source.y, 1)?;
    mouse.down(MouseButton::Left, 1)?;
    observer
        .watch(source_frame_id)
        .map_err(MouseError::Protocol)?;
    let trigger = MainFrameCssPoint {
        x: source.x + DRAG_START_DISTANCE,
        y: source.y,
    };
    mouse.move_to(trigger.x, trigger.y, 1)?;
    mouse.move_to(target.x, target.y, steps.max(DRAG_MOVE_STEPS))?;
    mouse.up(MouseButton::Left, 1)?;
    if !observer.settle().map_err(MouseError::Protocol)? {
        return Err(MouseError::Protocol(
            "Element did not start an HTML5 drag".to_string(),
        ));
    }
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
        Watch,
        Settle,
    }

    #[derive(Clone)]
    struct RecordingDriver {
        events: Arc<Mutex<Vec<Recorded>>>,
        started_drag: bool,
    }

    impl MouseDispatcher for RecordingDriver {
        fn dispatch(&mut self, event: crate::MouseDispatch) -> Result<(), String> {
            self.events.lock().unwrap().push(Recorded::Mouse(event));
            Ok(())
        }
    }

    impl DragObserver for RecordingDriver {
        fn watch(&mut self, _source_frame_id: &str) -> Result<(), String> {
            self.events.lock().unwrap().push(Recorded::Watch);
            Ok(())
        }

        fn settle(&mut self) -> Result<bool, String> {
            self.events.lock().unwrap().push(Recorded::Settle);
            Ok(self.started_drag)
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

    fn run_drag(started_drag: bool) -> (Vec<Recorded>, Result<(), MouseError>, MainFrameCssPoint) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let driver = RecordingDriver {
            events: events.clone(),
            started_drag,
        };
        let keyboard = Keyboard::new(NoopKeyboard);
        let mut mouse = Mouse::new(driver.clone(), &keyboard);
        let mut observer = driver;

        let result = drag_and_drop(
            &mut mouse,
            &mut observer,
            "main",
            MainFrameCssPoint { x: 10.0, y: 20.0 },
            MainFrameCssPoint { x: 30.0, y: 40.0 },
            1,
        );

        let recorded = events.lock().unwrap().clone();
        let position = mouse.position();
        (recorded, result, position)
    }

    fn position_of(
        events: &[Recorded],
        predicate: impl Fn(&crate::MouseEventPayload) -> bool,
    ) -> usize {
        events
            .iter()
            .position(|event| match event {
                Recorded::Mouse(crate::MouseDispatch::Mouse(payload)) => predicate(payload),
                _ => false,
            })
            .unwrap()
    }

    #[test]
    fn native_drag_presses_watches_crosses_the_threshold_then_releases_on_the_target() {
        let (events, result, position) = run_drag(true);

        result.unwrap();
        let pressed = position_of(&events, |payload| {
            payload.event_type == crate::MouseEventType::Pressed
        });
        let watch = events
            .iter()
            .position(|event| *event == Recorded::Watch)
            .unwrap();
        let trigger = position_of(&events, |payload| {
            payload.event_type == crate::MouseEventType::Moved
                && payload.x == 20.0
                && payload.y == 20.0
                && payload.buttons == Some(1)
        });
        let released = position_of(&events, |payload| {
            payload.event_type == crate::MouseEventType::Released
                && payload.x == 30.0
                && payload.y == 40.0
        });
        let read = events
            .iter()
            .position(|event| *event == Recorded::Settle)
            .unwrap();

        assert!(pressed < watch);
        assert!(watch < trigger);
        assert!(trigger < released);
        assert!(released < read);
        assert_eq!(position, MainFrameCssPoint { x: 30.0, y: 40.0 });
    }

    #[test]
    fn native_drag_travels_to_the_target_in_several_interpolated_moves() {
        let (events, _, _) = run_drag(true);

        let moves_to_target = events
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    Recorded::Mouse(crate::MouseDispatch::Mouse(payload))
                        if payload.event_type == crate::MouseEventType::Moved && payload.y > 20.0
                )
            })
            .count();

        assert_eq!(moves_to_target, DRAG_MOVE_STEPS);
    }

    #[test]
    fn drag_that_never_starts_releases_the_button_and_reports_it() {
        let (events, result, _) = run_drag(false);

        let error = result.unwrap_err();
        assert!(
            matches!(&error, MouseError::Protocol(message) if message == "Element did not start an HTML5 drag")
        );
        assert!(events.iter().any(|event| matches!(
            event,
            Recorded::Mouse(crate::MouseDispatch::Mouse(payload))
                if payload.event_type == crate::MouseEventType::Released
        )));
    }
}
