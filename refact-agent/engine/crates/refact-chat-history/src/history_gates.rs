//! Plain-fact gates for engine functions that change another chat: the opaque history pin, the
//! turn origin latched when a turn starts, who may be targeted, when a chat is idle enough to
//! change, and the order of a cascading delete. The engine supplies the facts; nothing here
//! reads a session, a file or a registry.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write;
use std::path::Path;

use refact_chat_api::ChatCommand;
use refact_core::chat_types::ChatMessage;
use sha2::{Digest, Sha256};

/// Request ids of commands an engine function queues; recognising them can only lower privilege.
pub const ENGINE_REQUEST_PREFIX: &str = "engine-";
/// Request ids of the regenerate the queue processor makes when deliveries wake a chat.
pub const DELIVERY_WAKE_PREFIX: &str = "delivery-wake-";
/// Chats one cascading delete may remove.
pub const MAX_CASCADE: usize = 50;

struct HashWriter<'a>(&'a mut Sha256);

impl Write for HashWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.update(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The opaque pin of one chat's history, bound to the chat id, the file that stores it and every
/// stored message. Hashed while serializing, so no second copy of the history is built.
pub fn history_pin(chat_id: &str, file: Option<&Path>, messages: &[ChatMessage]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"refact.history_pin.1\0");
    hasher.update(chat_id.as_bytes());
    hasher.update(b"\0");
    if let Some(file) = file {
        hasher.update(file.to_string_lossy().as_bytes());
    }
    hasher.update(b"\0");
    serde_json::to_writer(HashWriter(&mut hasher), messages).expect("chat messages serialize");
    format!("h1-{}", hex::encode(hasher.finalize()))
}

/// Who started a chat's current turn, latched by the engine when the turn starts and kept until
/// the next start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnOrigin {
    /// `None` when a person typed the message that started the turn; otherwise what did.
    pub automation: Option<String>,
    pub at_ms: u64,
}

/// The origin a queued command latches, or `None` when the chat keeps its latched origin: the
/// command starts no new turn (a recovery retry or a tool decision continues the current one), or
/// it is a regenerate from outside the engine of a turn whose origin is latched. A person is only
/// the author of a user message, a retry or an edit with regenerate that carries typed content
/// from outside the engine. `delivery_source` names the newest delivery for a wake.
pub fn turn_origin_of(
    request_id: &str,
    command: &ChatCommand,
    current: Option<&TurnOrigin>,
    delivery_source: Option<&str>,
    now_ms: u64,
) -> Option<TurnOrigin> {
    let engine = request_id.starts_with(ENGINE_REQUEST_PREFIX);
    let automation = match command {
        ChatCommand::UserMessage {
            ..
        }
        | ChatCommand::RetryFromIndex {
            ..
        }
        | ChatCommand::UpdateMessage {
            regenerate: true, ..
        } if !engine => None,
        ChatCommand::UserMessage {
            ..
        }
        | ChatCommand::RetryFromIndex {
            ..
        }
        | ChatCommand::UpdateMessage {
            regenerate: true, ..
        }
        | ChatCommand::Regenerate {}
            if engine =>
        {
            Some("an engine function of a script".to_string())
        }
        ChatCommand::Regenerate {} if request_id.starts_with(DELIVERY_WAKE_PREFIX) => {
            Some(match delivery_source {
                Some(source) => format!("a delivery from {source}"),
                None => "a delivery".to_string(),
            })
        }
        ChatCommand::Regenerate {} if current.is_some() => return None,
        ChatCommand::Regenerate {} => {
            Some("a regenerate of a turn whose origin is unknown".to_string())
        }
        ChatCommand::RemoveMessage {
            regenerate: true, ..
        } => Some("a regenerate after a removed message".to_string()),
        ChatCommand::DeliverMessages {
            delivery,
        } if delivery.wake => Some(format!("a delivery from {}", delivery.source)),
        _ => return None,
    };
    Some(TurnOrigin {
        automation,
        at_ms: now_ms,
    })
}

/// Why a python run may not raise engine change cards, given the origin its chat latched and
/// when the run started; `None` when a person started the turn the run belongs to.
pub fn engine_change_origin_refusal(
    latched: Option<&TurnOrigin>,
    run_started_ms: Option<u64>,
) -> Option<String> {
    let Some(origin) = latched else {
        return Some(
            "no turn has started in this chat since it was loaded (for example after an engine restart), so who started this run is unknown"
                .into(),
        );
    };
    let Some(started) = run_started_ms else {
        return Some("this run is not registered with its chat".into());
    };
    if started < origin.at_ms {
        return Some("this run started in an earlier turn of this chat".into());
    }
    origin
        .automation
        .as_ref()
        .map(|source| format!("this turn was started by {source}"))
}

/// How a refusal reaches the script: TypeError, PermissionError or ok=false.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    Type(String),
    Permission(String),
    Failed(String),
}

/// Who owns a target chat, as far as the engine could read it. `None` means it could not be read,
/// which refuses like an owner would.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TargetFacts {
    pub this_chat: bool,
    pub caller_is_child: bool,
    pub background_agent: bool,
    pub supervised: Option<bool>,
    pub runner_owned: bool,
    pub companion: bool,
    pub buddy: bool,
    pub task_role: Option<String>,
    pub closed: bool,
    /// `Some(true)` when both chats are stored in one project; `None` when a project is unknown.
    pub same_project: Option<bool>,
}

/// The one target-authority check of every function that changes another chat, run before its
/// snapshot and again before it applies.
pub fn target_refusal(chat_id: &str, facts: &TargetFacts) -> Option<Refusal> {
    if facts.this_chat {
        return Some(Refusal::Type(
            "this is this chat; use chat.* for this chat".into(),
        ));
    }
    if facts.caller_is_child {
        return Some(Refusal::Permission(
            "chats.* changes run only in a root chat".into(),
        ));
    }
    let refused = |why: String| Some(Refusal::Permission(format!("chat {chat_id} {why}")));
    if facts.background_agent {
        return refused("is a background agent's chat; its runner owns its history (continue and recover cannot be fenced against a script change), use the agents area".into());
    }
    match facts.supervised {
        Some(true) => return refused("runs under a live supervision grant".into()),
        None => {
            return refused("may run under a supervision grant, which cannot be read now".into())
        }
        Some(false) => {}
    }
    if facts.runner_owned {
        return refused("is owned by a background runner".into());
    }
    if facts.companion || facts.buddy {
        return refused("is a buddy conversation".into());
    }
    match facts.task_role.as_deref() {
        Some("planner") => {
            return refused("is a task planner chat; whether its board runs an operation for it cannot be checked without side effects, so scripts do not change it".into())
        }
        Some(role) => return refused(format!("belongs to a task (role {role}); its board owns it")),
        None => {}
    }
    if facts.closed {
        return Some(Refusal::Failed(format!("chat {chat_id} is closed")));
    }
    match facts.same_project {
        Some(true) => None,
        Some(false) => refused(
            "is stored under another project; chats.* changes work on chats of this project only"
                .into(),
        ),
        None => refused(
            "is not stored in a project folder of this engine, so its project cannot be told"
                .into(),
        ),
    }
}

/// What the idle gate reads from a chat. A saved chat that is not loaded has only the persisted
/// parts (pending deliveries, questions, recovery, wake).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IdleFacts {
    pub state: String,
    pub turn_finished: bool,
    pub streaming: bool,
    pub paused: bool,
    pub live_tool_calls: bool,
    pub compressing: bool,
    pub handoff_or_rebuild: bool,
    pub browser_decision: bool,
    pub interrupted: bool,
    pub planner_paused: bool,
    pub queued_commands: usize,
    pub taken_commands: usize,
    pub native_in_flight: usize,
    pub pending_deliveries: usize,
    pub wake_pending: bool,
    pub review_or_verifier: bool,
    pub recovery_pending: bool,
    pub external_reload: bool,
    pub open_tool_window: bool,
}

/// Why a chat is not idle enough for its history to change, naming the state.
pub fn idle_refusal(chat_id: &str, facts: &IdleFacts) -> Option<String> {
    let why = if !facts.turn_finished || facts.streaming {
        format!("is busy ({})", facts.state)
    } else if facts.paused {
        "is paused waiting for a decision".to_string()
    } else if facts.live_tool_calls {
        "is waiting for tool results or answers".to_string()
    } else if facts.compressing {
        "is compressing its history".to_string()
    } else if facts.handoff_or_rebuild {
        "has a mode handoff or context rebuild pending".to_string()
    } else if facts.browser_decision {
        "is waiting for a browser decision".to_string()
    } else if facts.interrupted || facts.planner_paused {
        "is held by its task board".to_string()
    } else if facts.queued_commands > 0 {
        format!("has {} queued commands", facts.queued_commands)
    } else if facts.taken_commands > 0 || facts.native_in_flight > 0 {
        "is about to run a command".to_string()
    } else if facts.pending_deliveries > 0 {
        format!("has {} pending deliveries", facts.pending_deliveries)
    } else if facts.wake_pending {
        "has a wake pending".to_string()
    } else if facts.review_or_verifier {
        "has a review or goal check pending".to_string()
    } else if facts.recovery_pending {
        "has a recovery retry pending".to_string()
    } else if facts.external_reload {
        "has a change from disk waiting to load".to_string()
    } else if facts.open_tool_window {
        "has tool calls waiting for results".to_string()
    } else {
        return None;
    };
    Some(format!("chat {chat_id} {why}; try again when it is idle"))
}

/// The chats a cascading delete of `root` removes, children before their parents, from the
/// parent link of every known chat. Refuses a member known twice, a cycle and more than `limit`.
pub fn cascade_order(
    root: &str,
    links: &[(String, Option<String>)],
    limit: usize,
) -> Result<Vec<String>, String> {
    let mut copies: HashMap<&str, usize> = HashMap::new();
    let mut children: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (id, parent) in links {
        *copies.entry(id.as_str()).or_default() += 1;
        if let Some(parent) = parent {
            children
                .entry(parent.as_str())
                .or_default()
                .push(id.as_str());
        }
    }
    for list in children.values_mut() {
        list.sort_unstable();
        list.dedup();
    }
    let mut order = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    let mut stack: Vec<(&str, bool)> = vec![(root, false)];
    while let Some((id, expanded)) = stack.pop() {
        if expanded {
            order.push(id.to_string());
            continue;
        }
        if copies.get(id).is_some_and(|count| *count > 1) {
            return Err(format!(
                "chat {id} has more than one saved copy; delete it by hand"
            ));
        }
        if !seen.insert(id) {
            return Err(format!("chat {id} is its own ancestor; delete it by hand"));
        }
        if seen.len() > limit {
            return Err(format!(
                "the chat tree of {root} holds more than {limit} chats; delete it in parts"
            ));
        }
        stack.push((id, true));
        if let Some(list) = children.get(id) {
            for child in list.iter().rev() {
                stack.push((*child, false));
            }
        }
    }
    Ok(order)
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_core::chat_types::{ChatContent, PendingDelivery, PushMode};
    use serde_json::json;

    fn msg(id: &str, role: &str, text: &str) -> ChatMessage {
        ChatMessage {
            message_id: id.into(),
            role: role.into(),
            content: ChatContent::SimpleText(text.into()),
            ..Default::default()
        }
    }

    fn command(value: serde_json::Value) -> ChatCommand {
        serde_json::from_value(value).expect("command parses")
    }

    #[test]
    fn the_pin_changes_with_any_message_field_the_chat_and_the_file() {
        let history = vec![msg("a", "user", "hello"), msg("b", "assistant", "hi")];
        let path = Path::new("/p/.refact/trajectories/x.json");
        let pin = history_pin("x", Some(path), &history);
        assert!(pin.starts_with("h1-"), "{pin}");
        assert_eq!(pin, history_pin("x", Some(path), &history.clone()));
        let mut edited = history.clone();
        edited[1].content = ChatContent::SimpleText("hi!".into());
        assert_ne!(pin, history_pin("x", Some(path), &edited));
        let mut extra = history.clone();
        extra[0]
            .extra
            .insert("privacy".into(), json!({"files": []}));
        assert_ne!(pin, history_pin("x", Some(path), &extra));
        assert_ne!(pin, history_pin("y", Some(path), &history));
        assert_ne!(
            pin,
            history_pin(
                "x",
                Some(Path::new("/q/.refact/trajectories/x.json")),
                &history
            )
        );
        assert_ne!(pin, history_pin("x", None, &history));
        assert_ne!(pin, history_pin("x", Some(path), &history[..1]));
    }

    #[test]
    fn only_typed_content_from_outside_the_engine_latches_a_person() {
        let person = TurnOrigin {
            automation: None,
            at_ms: 3,
        };
        let wake = TurnOrigin {
            automation: Some("a delivery from scheduler.cron".into()),
            at_ms: 3,
        };
        let user = command(json!({"type": "user_message", "content": "hi"}));
        let retry = command(json!({"type": "retry_from_index", "index": 1, "content": "hi"}));
        let edit = command(
            json!({"type": "update_message", "message_id": "m", "content": "hi", "regenerate": true}),
        );
        for typed in [&user, &retry, &edit] {
            for current in [None, Some(&person), Some(&wake)] {
                assert_eq!(
                    turn_origin_of("gui-1", typed, current, None, 7),
                    Some(TurnOrigin {
                        automation: None,
                        at_ms: 7
                    })
                );
                let engine = turn_origin_of("engine-1", typed, current, None, 7).unwrap();
                assert_eq!(
                    engine.automation.as_deref(),
                    Some("an engine function of a script"),
                    "an http client using the engine prefix gains nothing"
                );
                assert_eq!(engine.at_ms, 7);
            }
        }
        let regenerate = command(json!({"type": "regenerate"}));
        for current in [Some(&person), Some(&wake)] {
            assert_eq!(
                turn_origin_of("gui-2", &regenerate, current, None, 9),
                None,
                "a regenerate from outside the engine keeps the origin of the turn it regenerates"
            );
        }
        assert_eq!(
            turn_origin_of("gui-2", &regenerate, None, None, 9)
                .unwrap()
                .automation
                .as_deref(),
            Some("a regenerate of a turn whose origin is unknown")
        );
        for current in [None, Some(&person)] {
            assert_eq!(
                turn_origin_of("engine-2", &regenerate, current, None, 9)
                    .unwrap()
                    .automation
                    .as_deref(),
                Some("an engine function of a script")
            );
            assert_eq!(
                turn_origin_of(
                    "delivery-wake-3",
                    &regenerate,
                    current,
                    Some("scheduler.cron"),
                    9
                )
                .unwrap()
                .automation
                .as_deref(),
                Some("a delivery from scheduler.cron")
            );
            assert_eq!(
                turn_origin_of("delivery-wake-3", &regenerate, current, None, 9)
                    .unwrap()
                    .automation
                    .as_deref(),
                Some("a delivery")
            );
        }
        let remove =
            command(json!({"type": "remove_message", "message_id": "m", "regenerate": true}));
        assert!(turn_origin_of("gui-4", &remove, Some(&person), None, 1)
            .unwrap()
            .automation
            .is_some());
        let delivery = PendingDelivery::new(
            vec![msg("d", "user", "scheduled")],
            PushMode::WhenIdle,
            "scheduler.cron",
            true,
        );
        let deliver = ChatCommand::DeliverMessages {
            delivery: delivery.clone(),
        };
        assert_eq!(
            turn_origin_of("x", &deliver, Some(&person), None, 1)
                .unwrap()
                .automation
                .as_deref(),
            Some("a delivery from scheduler.cron")
        );
        let quiet = ChatCommand::DeliverMessages {
            delivery: PendingDelivery {
                wake: false,
                ..delivery
            },
        };
        for keeps in [
            quiet,
            command(json!({"type": "update_message", "message_id": "m", "content": "hi"})),
            command(json!({"type": "remove_message", "message_id": "m"})),
            command(json!({"type": "set_params", "patch": {}})),
            command(json!({"type": "tool_decision", "tool_call_id": "c", "accepted": true})),
            command(json!({"type": "recovery_retry_now", "run_id": "r", "expected_revision": 1})),
            command(json!({
                "type": "recovery_retry",
                "run_id": "r",
                "expected_revision": 1,
                "options": {"mode": "transient", "max_retries": 1, "delay_ms": 0, "total_timeout_ms": 1000}
            })),
        ] {
            for current in [None, Some(&person), Some(&wake)] {
                assert_eq!(
                    turn_origin_of("gui", &keeps, current, None, 1),
                    None,
                    "{keeps:?}"
                );
            }
        }
    }

    #[test]
    fn engine_changes_need_a_person_latched_before_the_run_started() {
        let person = TurnOrigin {
            automation: None,
            at_ms: 100,
        };
        assert_eq!(engine_change_origin_refusal(Some(&person), Some(100)), None);
        assert_eq!(engine_change_origin_refusal(Some(&person), Some(250)), None);
        assert!(engine_change_origin_refusal(Some(&person), Some(99))
            .unwrap()
            .contains("earlier turn"));
        assert!(engine_change_origin_refusal(Some(&person), None)
            .unwrap()
            .contains("not registered"));
        assert!(engine_change_origin_refusal(None, Some(100))
            .unwrap()
            .contains("engine restart"));
        let wake = TurnOrigin {
            automation: Some("a delivery from scheduler.cron".into()),
            at_ms: 100,
        };
        assert_eq!(
            engine_change_origin_refusal(Some(&wake), Some(150)).as_deref(),
            Some("this turn was started by a delivery from scheduler.cron")
        );
    }

    #[test]
    fn every_owner_and_an_unknown_custody_refuse_the_target() {
        let open = TargetFacts {
            supervised: Some(false),
            same_project: Some(true),
            ..Default::default()
        };
        assert_eq!(target_refusal("c", &open), None);
        let cases: Vec<(TargetFacts, &str)> = vec![
            (
                TargetFacts {
                    this_chat: true,
                    ..open.clone()
                },
                "type",
            ),
            (
                TargetFacts {
                    caller_is_child: true,
                    ..open.clone()
                },
                "root chat",
            ),
            (
                TargetFacts {
                    background_agent: true,
                    ..open.clone()
                },
                "background agent",
            ),
            (
                TargetFacts {
                    supervised: Some(true),
                    ..open.clone()
                },
                "supervision",
            ),
            (
                TargetFacts {
                    supervised: None,
                    ..open.clone()
                },
                "cannot be read",
            ),
            (
                TargetFacts {
                    runner_owned: true,
                    ..open.clone()
                },
                "runner",
            ),
            (
                TargetFacts {
                    companion: true,
                    ..open.clone()
                },
                "buddy",
            ),
            (
                TargetFacts {
                    buddy: true,
                    ..open.clone()
                },
                "buddy",
            ),
            (
                TargetFacts {
                    task_role: Some("agent".into()),
                    ..open.clone()
                },
                "board owns it",
            ),
            (
                TargetFacts {
                    task_role: Some("planner".into()),
                    ..open.clone()
                },
                "planner",
            ),
            (
                TargetFacts {
                    closed: true,
                    ..open.clone()
                },
                "closed",
            ),
            (
                TargetFacts {
                    same_project: Some(false),
                    ..open.clone()
                },
                "another project",
            ),
            (
                TargetFacts {
                    same_project: None,
                    ..open.clone()
                },
                "cannot be told",
            ),
        ];
        for (facts, expected) in cases {
            let refusal = target_refusal("c", &facts).expect("refused");
            match (&refusal, expected) {
                (Refusal::Type(_), "type") => {}
                (Refusal::Failed(text), _) => assert!(text.contains(expected), "{text}"),
                (Refusal::Permission(text), _) => assert!(text.contains(expected), "{text}"),
                other => panic!("{other:?} for {expected}"),
            }
        }
    }

    #[test]
    fn each_busy_state_is_named_and_an_idle_chat_passes() {
        let idle = IdleFacts {
            state: "idle".into(),
            turn_finished: true,
            ..Default::default()
        };
        assert_eq!(idle_refusal("c", &idle), None);
        let cases: Vec<(IdleFacts, &str)> = vec![
            (
                IdleFacts {
                    turn_finished: false,
                    state: "generating".into(),
                    ..idle.clone()
                },
                "busy (generating)",
            ),
            (
                IdleFacts {
                    streaming: true,
                    ..idle.clone()
                },
                "busy",
            ),
            (
                IdleFacts {
                    paused: true,
                    ..idle.clone()
                },
                "paused",
            ),
            (
                IdleFacts {
                    live_tool_calls: true,
                    ..idle.clone()
                },
                "tool results or answers",
            ),
            (
                IdleFacts {
                    compressing: true,
                    ..idle.clone()
                },
                "compressing",
            ),
            (
                IdleFacts {
                    handoff_or_rebuild: true,
                    ..idle.clone()
                },
                "handoff",
            ),
            (
                IdleFacts {
                    browser_decision: true,
                    ..idle.clone()
                },
                "browser",
            ),
            (
                IdleFacts {
                    interrupted: true,
                    ..idle.clone()
                },
                "board",
            ),
            (
                IdleFacts {
                    planner_paused: true,
                    ..idle.clone()
                },
                "board",
            ),
            (
                IdleFacts {
                    queued_commands: 2,
                    ..idle.clone()
                },
                "2 queued commands",
            ),
            (
                IdleFacts {
                    taken_commands: 1,
                    ..idle.clone()
                },
                "about to run a command",
            ),
            (
                IdleFacts {
                    native_in_flight: 1,
                    ..idle.clone()
                },
                "about to run a command",
            ),
            (
                IdleFacts {
                    pending_deliveries: 1,
                    ..idle.clone()
                },
                "1 pending deliveries",
            ),
            (
                IdleFacts {
                    wake_pending: true,
                    ..idle.clone()
                },
                "wake",
            ),
            (
                IdleFacts {
                    review_or_verifier: true,
                    ..idle.clone()
                },
                "goal check",
            ),
            (
                IdleFacts {
                    recovery_pending: true,
                    ..idle.clone()
                },
                "recovery",
            ),
            (
                IdleFacts {
                    external_reload: true,
                    ..idle.clone()
                },
                "from disk",
            ),
            (
                IdleFacts {
                    open_tool_window: true,
                    ..idle.clone()
                },
                "waiting for results",
            ),
        ];
        for (facts, expected) in cases {
            let refusal = idle_refusal("c", &facts).unwrap_or_else(|| panic!("{expected} passed"));
            assert!(refusal.contains(expected), "{refusal}");
            assert!(refusal.starts_with("chat c "), "{refusal}");
        }
    }

    #[test]
    fn a_cascade_deletes_children_first_and_refuses_ambiguity_cycles_and_size() {
        let links = |pairs: &[(&str, Option<&str>)]| -> Vec<(String, Option<String>)> {
            pairs
                .iter()
                .map(|(id, parent)| (id.to_string(), parent.map(str::to_string)))
                .collect()
        };
        let tree = links(&[
            ("root", None),
            ("b", Some("root")),
            ("a", Some("root")),
            ("a1", Some("a")),
            ("other", None),
            ("o1", Some("other")),
        ]);
        assert_eq!(
            cascade_order("root", &tree, MAX_CASCADE).unwrap(),
            ["a1", "a", "b", "root"]
        );
        assert_eq!(cascade_order("a1", &tree, MAX_CASCADE).unwrap(), ["a1"]);
        assert_eq!(
            cascade_order("unknown", &tree, MAX_CASCADE).unwrap(),
            ["unknown"]
        );
        assert!(cascade_order("root", &tree, 3)
            .unwrap_err()
            .contains("more than 3 chats"));
        let twice = links(&[("root", None), ("a", Some("root")), ("a", Some("other"))]);
        assert!(cascade_order("root", &twice, MAX_CASCADE)
            .unwrap_err()
            .contains("more than one saved copy"));
        let twice_elsewhere = links(&[
            ("root", None),
            ("a", Some("root")),
            ("x", None),
            ("x", None),
        ]);
        assert_eq!(
            cascade_order("root", &twice_elsewhere, MAX_CASCADE).unwrap(),
            ["a", "root"]
        );
        assert!(cascade_order("x", &twice_elsewhere, MAX_CASCADE)
            .unwrap_err()
            .contains("more than one saved copy"));
        let cycle = links(&[("x", Some("y")), ("y", Some("x"))]);
        assert!(cascade_order("x", &cycle, MAX_CASCADE)
            .unwrap_err()
            .contains("own ancestor"));
    }
}
