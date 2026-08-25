use serde_json::{json, Value};

use crate::history::cells::tool_display_name;
use crate::sessions::WorktreeMeta;

use super::transcript::{
    citation_item, finalized_assistant_content_part, is_plan_delta_message, render_message_key,
    rendered_state_keys_for_message, server_content_block_item, session_header_key,
    state_key_has_stable_identity,
};
use super::*;

impl App {
    pub(super) fn approval_scope(&self, raw: &Value, event_seq: Option<u64>) -> String {
        let pause_id = raw
            .get("pause_id")
            .or_else(|| raw.get("id"))
            .or_else(|| raw.get("message_id"))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        if let Some(pause_id) = pause_id {
            return format!("{}:{pause_id}", self.chat_id);
        }
        let tool_call_ids = approval_tool_call_ids(raw);
        if !tool_call_ids.is_empty() {
            return format!(
                "{}:tools:{:016x}",
                self.chat_id,
                stable_scope_hash(&tool_call_ids)
            );
        }
        match event_seq {
            Some(seq) => format!("{}:seq:{seq}", self.chat_id),
            None => self.chat_id.clone(),
        }
    }

    pub(super) fn explicit_approval_scope(&self, raw: &Value) -> Option<String> {
        let has_pause_id = raw
            .get("pause_id")
            .or_else(|| raw.get("id"))
            .or_else(|| raw.get("message_id"))
            .and_then(Value::as_str)
            .is_some_and(|value| !value.is_empty());
        if has_pause_id || !approval_tool_call_ids(raw).is_empty() {
            Some(self.approval_scope(raw, None))
        } else {
            None
        }
    }

    pub(super) fn queue_notification(&mut self, kind: NotificationKind) {
        self.notifications.queue(kind);
    }

    pub(super) fn set_terminal_focus(&mut self, focused: bool) {
        self.notifications.set_focused(focused);
    }

    pub(super) fn take_pending_notifications(&mut self) -> Vec<Vec<u8>> {
        self.notifications.drain_pending()
    }

    pub(super) fn notification_status_label(&self) -> &'static str {
        let config = self.notifications.config();
        if !config.enabled() {
            "off"
        } else if config.bell() {
            "OSC9 + BEL"
        } else {
            "OSC9"
        }
    }

    pub(super) fn clear_approvals(&mut self) {
        let ids = self.approval_queue.tool_call_ids();
        self.approval_queue.clear();
        self.pending_approval_clears.clear();
        for item in &mut self.transcript {
            if let TranscriptItem::Tool(card) = item {
                if ids.iter().any(|id| id == &card.id)
                    && card.status == ToolStatus::AwaitingApproval
                {
                    card.status = ToolStatus::Cancelled;
                    card.subchat_active = false;
                }
            }
        }
    }

    pub(super) fn set_tool_statuses(&mut self, ids: &[String], status: ToolStatus) {
        for item in &mut self.transcript {
            if let TranscriptItem::Tool(card) = item {
                if ids.iter().any(|id| id == &card.id) {
                    card.status = status;
                    if status.is_final() {
                        card.subchat_active = false;
                    }
                }
            }
        }
        let statuses = ids
            .iter()
            .cloned()
            .map(|id| (id, status))
            .collect::<Vec<_>>();
        if self.history.set_tool_statuses(&statuses)
            && self.native_scrollback
            && self.history.inserted_cell_count() > 0
        {
            self.resize_reflow.schedule_immediate();
        }
    }

    pub(super) fn enqueue_approval(&mut self, modal: ApprovalModalState) {
        if self.approval_scope_pending_clear(modal.scope()) {
            return;
        }
        let ids = modal.tool_call_ids().to_vec();
        if self.approval_queue.push(modal) {
            self.set_tool_statuses(&ids, ToolStatus::AwaitingApproval);
            self.queue_notification(NotificationKind::ApprovalNeeded);
        }
    }

    pub(super) fn pop_current_approval(&mut self) -> Option<ApprovalModalState> {
        let modal = self.approval_queue.pop_front();
        if let Some(modal) = &modal {
            self.mark_approval_pending_clear(modal);
        }
        modal
    }

    pub(super) fn approval_scope_pending_clear(&self, scope: &str) -> bool {
        self.pending_approval_clears
            .iter()
            .any(|pending| pending.scope == scope)
    }

    pub(super) fn mark_approval_pending_clear(&mut self, modal: &ApprovalModalState) {
        if !self.approval_scope_pending_clear(modal.scope()) {
            self.pending_approval_clears
                .push_back(PendingApprovalClear {
                    scope: modal.scope().to_string(),
                    tool_call_ids: modal.tool_call_ids().to_vec(),
                });
        }
        self.approval_queue.remove_scope(modal.scope());
    }

    pub(super) fn handle_pause_cleared(&mut self, raw: &Value) {
        if let Some(scope) = self.explicit_approval_scope(raw) {
            self.pending_approval_clears
                .retain(|pending| pending.scope != scope);
            self.approval_queue.remove_scope(&scope);
        } else {
            self.pending_approval_clears.pop_front();
        }
    }

    pub(super) fn pending_clear_tool_call_ids(&self) -> Vec<String> {
        let mut ids = self
            .pending_approval_clears
            .iter()
            .flat_map(|pending| pending.tool_call_ids.iter().cloned())
            .collect::<Vec<_>>();
        ids.sort();
        ids.dedup();
        ids
    }

    pub(super) fn filtered_approval_raw(&self, raw: &Value) -> Value {
        let pending_ids = self.pending_clear_tool_call_ids();
        if pending_ids.is_empty() {
            return raw.clone();
        }
        let mut filtered = raw.clone();
        let Some(map) = filtered.as_object_mut() else {
            return filtered;
        };
        for key in ["reasons", "pause_reasons"] {
            if let Some(Value::Array(reasons)) = map.get_mut(key) {
                reasons.retain(|reason| {
                    let reason_ids = approval_tool_call_ids(reason);
                    reason_ids.is_empty() || reason_ids.iter().any(|id| !pending_ids.contains(id))
                });
            }
        }
        filtered
    }

    pub fn apply_chat_event(&mut self, event: ChatEvent) -> AppAction {
        self.handle_chat_event(event)
    }

    pub fn apply_stream_commit_tick(&mut self) {
        self.run_stream_commit_tick();
    }

    pub(super) fn handle_chat_event(&mut self, event: ChatEvent) -> AppAction {
        if event
            .chat_id
            .as_deref()
            .is_some_and(|chat_id| chat_id != self.chat_id)
        {
            return AppAction::None;
        }
        self.daemon_online = true;
        self.subscription_status = SubscriptionStatus::Online;
        self.retry_hint = None;
        let protocol_event = event.protocol_event();
        let raw = event.raw;
        match protocol_event {
            SseEvent::Snapshot {
                background_agents,
                browser,
                ..
            } => {
                self.inbound_event_state
                    .apply_snapshot(background_agents, browser.clone());
                self.browser_state.apply_snapshot(browser);
                let action = self.handle_snapshot(&raw);
                self.refresh_activity_surface();
                return action;
            }
            SseEvent::BackgroundAgentUpdated { agent } => {
                self.inbound_event_state.update_background_agent(agent);
            }
            SseEvent::StreamStarted { message_id } => {
                self.set_session_state(SessionState::Generating);
                self.clear_stream_controllers();
                self.transcript_state.start_assistant(message_id.as_deref());
                self.rebuild_render_transcript_from_state();
            }
            SseEvent::StreamDelta { message_id, ops } => {
                self.handle_stream_delta(message_id.as_deref(), &ops)
            }
            SseEvent::MalformedStreamDelta { message_id, reason } => {
                let message = message_id
                    .as_deref()
                    .map(|id| format!(" for {id}"))
                    .unwrap_or_default();
                self.add_notice(format!(
                    "Rejected malformed stream_delta{message}: {reason}"
                ));
            }
            SseEvent::StreamFinished {
                message_id, usage, ..
            } => {
                self.transcript_state
                    .finish_assistant(message_id.as_deref(), usage.clone());
                let reasoning_key = self
                    .finalized_assistant_message(message_id.as_deref())
                    .and_then(|message| {
                        (!message.reasoning.is_empty()).then(|| {
                            (
                                render_message_key(message, "reasoning", 0),
                                message.reasoning.clone(),
                            )
                        })
                    });
                if let Some((key, reasoning)) = reasoning_key {
                    self.push_state_reasoning_item(
                        key,
                        reasoning,
                        true,
                        self.reasoning_stream_active,
                    );
                }
                let final_content = self.finalize_assistant_stream();
                if self.native_scrollback {
                    if final_content.is_some() {
                        if let Some(message) =
                            self.finalized_assistant_message(message_id.as_deref())
                        {
                            let key = render_message_key(
                                message,
                                "assistant",
                                finalized_assistant_content_part(message),
                            );
                            self.record_state_history_key(key);
                        }
                    }
                    self.schedule_final_stream_resize_reflow();
                }
                self.finalize_tool_cards_for_turn();
                if let Some(usage) = usage {
                    self.update_usage_value(&usage);
                } else {
                    self.update_usage(&raw);
                }
                self.queue_notification(NotificationKind::TurnComplete);
                if matches!(
                    self.session_state,
                    SessionState::Generating | SessionState::ExecutingTools
                ) {
                    if self.ask_questions_form.is_some() {
                        self.set_session_state(SessionState::WaitingUserInput);
                    } else {
                        self.set_session_state(SessionState::Idle);
                    }
                }
            }
            SseEvent::RuntimeUpdated { runtime } => {
                self.runtime_snapshot = Some(runtime);
                return self.handle_runtime_updated(&raw);
            }
            SseEvent::QueueUpdated {
                queue_size,
                queued_items,
            } => self.update_server_queue(queue_size, queued_items),
            SseEvent::PauseRequired => self.handle_pause_required(&raw, event.seq),
            SseEvent::PauseCleared => self.handle_pause_cleared(&raw),
            SseEvent::ThreadUpdated { params } => self.handle_thread_updated(&params),
            SseEvent::MessageAdded { message, index } => {
                self.handle_message_added_payload(message.as_ref(), index)
            }
            SseEvent::MessageUpdated {
                message_id,
                message,
            } => self.handle_message_updated_payload(message_id.as_deref(), message.as_ref()),
            SseEvent::MessageRemoved { message_id } => {
                self.handle_message_removed(message_id.as_deref())
            }
            SseEvent::MessagesTruncated { from_index } => {
                self.handle_messages_truncated(from_index)
            }
            SseEvent::SubchatUpdate {
                tool_call_id,
                subchat_id,
                attached_files,
                depth,
            } => self.handle_subchat_update(&tool_call_id, &subchat_id, &attached_files, depth),
            SseEvent::Ack {
                client_request_id,
                accepted,
                ..
            } => self.handle_send_ack(&client_request_id, accepted),
            SseEvent::ProcessCompleted { event } => {
                self.inbound_event_state.set_process_completed(event);
            }
            SseEvent::IdeToolRequired { event } => {
                self.inbound_event_state.set_ide_tool_required(event);
            }
            SseEvent::BrowserFrame { event } => {
                self.inbound_event_state.set_browser_frame(event.clone());
                self.browser_state.apply_frame(event);
            }
            SseEvent::BrowserStatus { event } => {
                self.inbound_event_state.set_browser_status(event.clone());
                self.browser_state.apply_status(event.snapshot);
            }
            SseEvent::BrowserClosed { event } => {
                self.inbound_event_state.set_browser_closed(event.clone());
                self.browser_state.apply_closed(event);
            }
            SseEvent::BrowserTimeline { event } => {
                self.inbound_event_state.set_browser_timeline(event.clone());
                self.browser_state.apply_timeline(event.events);
            }
            SseEvent::BrowserContextOversize { event } => {
                self.inbound_event_state
                    .set_browser_context_oversize(event.clone());
                self.browser_state.apply_context_oversize(event);
            }
            SseEvent::BrowserToolbarAction { event } => {
                self.inbound_event_state
                    .set_browser_toolbar_action(event.clone());
                self.browser_state.apply_toolbar_action(event.action);
            }
            SseEvent::Unknown { event } => {
                let kind = if event.kind.is_empty() {
                    "(missing type)"
                } else {
                    event.kind.as_str()
                };
                self.add_notice(format!("Unknown SSE event: {kind}"));
                self.inbound_event_state.record_unknown(event);
            }
        }
        self.refresh_activity_surface();
        AppAction::None
    }

    pub(super) fn handle_thread_updated(&mut self, raw: &Value) {
        let params = thread_update_params(raw);
        self.update_worktree_meta(params);
        if let Some(title) = params
            .get("title")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        {
            self.session_title = Some(title.to_string());
            self.show_session_header = true;
        }
        if params.get("model").is_some() {
            if let Some(model) = params
                .get("model")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
            {
                self.model = Some(model.to_string());
            }
        }
        if params.get("mode").is_some() || params.get("tool_use").is_some() {
            if let Some(mode) = params
                .get("mode")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .or_else(|| {
                    params
                        .get("tool_use")
                        .and_then(Value::as_str)
                        .filter(|value| !value.is_empty())
                })
            {
                self.mode = Some(mode.to_string());
            }
        }
        if let Some(task_id) = task_id_from_thread(params) {
            self.task_id = Some(task_id);
        }
        if let Some(value) = params.get("boost_reasoning").and_then(Value::as_bool) {
            self.boost_reasoning = value;
        }
        if params.get("reasoning_effort").is_some() {
            self.reasoning_effort = params
                .get("reasoning_effort")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
        }
        if let Some(value) = params
            .get("auto_approve_editing_tools")
            .and_then(Value::as_bool)
        {
            self.permission_policy.auto_approve_editing_tools = value;
        }
        if let Some(value) = params
            .get("auto_approve_dangerous_commands")
            .and_then(Value::as_bool)
        {
            self.permission_policy.auto_approve_dangerous_commands = value;
        }
        self.update_thread_params(&params);
        self.refresh_session_header_item();
    }

    pub(super) fn handle_runtime_updated(&mut self, raw: &Value) -> AppAction {
        let became_idle = self.apply_runtime_state(raw);
        self.maybe_open_pending_ask_questions_form();
        self.update_server_queue_from_runtime(raw);
        self.sync_runtime_approvals(raw);
        if became_idle {
            self.dispatch_next_queued_input()
        } else {
            AppAction::None
        }
    }

    pub(super) fn sync_runtime_approvals(&mut self, runtime: &Value) {
        let paused_tool_call_ids = approval_tool_call_ids(runtime);
        if approval_reasons_present(runtime) {
            self.retain_pending_clears_still_paused(&paused_tool_call_ids);
        }
        let filtered_runtime = self.filtered_approval_raw(runtime);
        if let Some(modal) = ApprovalModalState::from_event_in_scope(
            self.approval_scope(&filtered_runtime, None),
            &filtered_runtime,
        ) {
            self.enqueue_approval(modal);
        } else if self.session_state != SessionState::Paused
            || (approval_reasons_present(runtime) && paused_tool_call_ids.is_empty())
        {
            self.clear_approvals();
        }
    }

    pub(super) fn retain_pending_clears_still_paused(&mut self, paused_tool_call_ids: &[String]) {
        if paused_tool_call_ids.is_empty() {
            return;
        }
        self.pending_approval_clears.retain(|pending| {
            pending.tool_call_ids.is_empty()
                || pending
                    .tool_call_ids
                    .iter()
                    .any(|id| paused_tool_call_ids.contains(id))
        });
    }

    pub(super) fn update_server_queue_from_runtime(&mut self, raw: &Value) {
        if raw.get("queue_size").is_none() && raw.get("queued_items").is_none() {
            return;
        }
        let queue_size = raw
            .get("queue_size")
            .and_then(Value::as_u64)
            .unwrap_or_default() as usize;
        let queued_items = raw
            .get("queued_items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        self.update_server_queue(queue_size, queued_items);
    }

    pub(super) fn update_server_queue(&mut self, queue_size: usize, queued_items: Vec<Value>) {
        self.server_queue_size = queue_size;
        self.server_queue_previews = queued_items
            .iter()
            .filter_map(|item| item.get("preview").and_then(Value::as_str))
            .map(str::to_string)
            .collect();
    }

    pub(super) fn update_usage(&mut self, raw: &Value) {
        if let Some(usage) = raw
            .get("usage")
            .or_else(|| raw.get("last_usage"))
            .or_else(|| raw.get("token_usage"))
        {
            self.update_usage_value(usage);
        }
    }

    pub(super) fn update_usage_value(&mut self, usage: &Value) {
        self.transcript_state.set_usage(usage.clone());
        self.usage = UsageSummary::from_value(usage);
    }

    pub(super) fn apply_runtime_state(&mut self, raw: &Value) -> bool {
        if raw
            .get("error")
            .and_then(Value::as_str)
            .is_some_and(|error| !error.is_empty())
        {
            self.set_session_state(SessionState::Error);
            return false;
        }
        let Some(runtime_state) = raw.get("state").and_then(Value::as_str) else {
            self.add_notice("Runtime update omitted state; preserving current state");
            return false;
        };
        let state = match runtime_state {
            "idle" => SessionState::Idle,
            "generating" => SessionState::Generating,
            "executing_tools" => SessionState::ExecutingTools,
            "paused" => SessionState::Paused,
            "waiting_ide" => SessionState::WaitingIde,
            "waiting_user_input" => SessionState::WaitingUserInput,
            "completed" => SessionState::Completed,
            "error" => SessionState::Error,
            _ => {
                self.add_notice(format!(
                    "Unknown runtime state {runtime_state:?}; preserving current state"
                ));
                return false;
            }
        };
        self.set_session_state(state);
        runtime_state == "idle"
    }

    pub(super) fn maybe_open_pending_ask_questions_form(&mut self) {
        if self.session_state != SessionState::WaitingUserInput || self.ask_questions_form.is_some()
        {
            return;
        }
        let pending = self
            .transcript_state
            .messages()
            .iter()
            .rev()
            .take_while(|message| message.role != TranscriptRole::User)
            .find(|message| message.role.is_tool_result() && !message.tool_failed)
            .cloned();
        if let Some(message) = pending.as_ref() {
            self.maybe_open_ask_questions_form(message);
        }
    }

    pub(super) fn handle_pause_required(&mut self, raw: &Value, event_seq: Option<u64>) {
        self.set_session_state(SessionState::Paused);
        let filtered = self.filtered_approval_raw(raw);
        match ApprovalModalState::from_event_in_scope(
            self.approval_scope(&filtered, event_seq),
            &filtered,
        ) {
            Some(modal) => self.enqueue_approval(modal),
            None if !approval_reasons_present(raw) => {
                self.add_notice("Approval required but no tool metadata was provided")
            }
            None => {}
        }
    }

    pub(super) fn handle_stream_delta(&mut self, message_id: Option<&str>, ops: &[DeltaOp]) {
        self.transcript_state.apply_delta_ops(message_id, ops);
        let mut thinking_blocks_changed = false;
        for op in ops {
            match op {
                DeltaOp::AppendContent { text } => self.append_assistant(text),
                DeltaOp::AppendReasoning { text } => self.append_reasoning(text),
                DeltaOp::SetReasoning { text } => {
                    self.replace_reasoning_stream_with_final(text.clone(), true)
                }
                DeltaOp::SetUsage { usage } => self.update_usage_value(usage),
                DeltaOp::AddCitation { citation } => {
                    self.push_history_item(citation_item(citation));
                }
                DeltaOp::AddServerContentBlock { block } => {
                    self.push_history_item(server_content_block_item(block));
                }
                DeltaOp::SetThinkingBlocks { .. } => thinking_blocks_changed = true,
                DeltaOp::SetToolCalls { tool_calls } => {
                    for tool in tool_calls {
                        self.push_tool_call(tool);
                    }
                }
                DeltaOp::MergeExtra { .. } => {}
                DeltaOp::Unknown(unknown) => {
                    let op = unknown.op.as_deref().unwrap_or("(missing op)");
                    self.add_notice(format!("Unknown stream_delta op: {op}"));
                }
            }
        }
        if thinking_blocks_changed {
            self.rebuild_render_transcript_from_state();
        }
    }

    pub(super) fn maybe_open_ask_questions_form(&mut self, message: &TranscriptMessage) {
        if self.session_state != SessionState::WaitingUserInput {
            return;
        }
        if message.tool_failed {
            return;
        }
        let request = AskQuestionsRequest::from_tool_content(
            &message.content,
            message.tool_call_id.as_deref(),
        );
        let Some(request) = request else {
            return;
        };
        if self
            .ask_questions_form
            .as_ref()
            .is_some_and(|form| form.tool_call_id() == request.tool_call_id)
        {
            return;
        }
        if self
            .handled_ask_questions_tool_ids
            .contains(&request.tool_call_id)
        {
            return;
        }
        if self.has_later_user_message_after_tool(&request.tool_call_id) {
            return;
        }
        let tool_call_id = request.tool_call_id.clone();
        self.ask_questions_form = Some(AskQuestionsForm::new(request));
        self.collapse_tool_card(&tool_call_id);
        self.session_state = SessionState::WaitingUserInput;
    }

    pub(super) fn collapse_tool_card(&mut self, tool_call_id: &str) {
        for item in &mut self.transcript {
            if let TranscriptItem::Tool(card) = item {
                if card.id == tool_call_id {
                    card.expanded = false;
                }
            }
        }
    }

    pub(super) fn has_later_user_message_after_tool(&self, tool_call_id: &str) -> bool {
        let mut seen_tool = false;
        for message in self.transcript_state.messages() {
            if seen_tool && message.role == TranscriptRole::User {
                return true;
            }
            if message.role.is_tool_result()
                && message.tool_call_id.as_deref() == Some(tool_call_id)
            {
                seen_tool = true;
            }
        }
        false
    }

    pub(super) fn push_tool_call(&mut self, tool: &Value) {
        let card = ToolCard::from_tool_call(tool);
        if !card.id.is_empty() {
            if let Some((idx, existing)) =
                self.transcript
                    .iter_mut()
                    .enumerate()
                    .find_map(|(idx, item)| match item {
                        TranscriptItem::Tool(existing) if existing.id == card.id => {
                            Some((idx, existing))
                        }
                        _ => None,
                    })
            {
                existing.update_from_tool_call(card);
                self.selected_tool_index = Some(idx);
                let detail = existing.summary();
                if self.session_state.shows_working_indicator() && !detail.is_empty() {
                    self.working_detail = Some(detail);
                }
                return;
            }
        }
        let detail = card.summary();
        self.push_live_item(TranscriptItem::Tool(card));
        self.selected_tool_index = Some(self.transcript.len() - 1);
        self.set_working_detail(detail);
    }

    pub(super) fn handle_subchat_update(
        &mut self,
        tool_call_id: &str,
        subchat_id: &str,
        attached_files: &[String],
        depth: usize,
    ) {
        if tool_call_id.is_empty() {
            return;
        }
        let depth = depth.clamp(1, MAX_SUBCHAT_DEPTH);
        let (progress, progress_truncated) = truncate_subchat_progress(subchat_id);
        let state_updated = self.update_state_subchat(
            tool_call_id,
            subchat_id,
            &progress,
            attached_files,
            depth,
            progress_truncated,
        );
        let updated = self.update_visible_subchat(
            tool_call_id,
            subchat_id,
            &progress,
            attached_files,
            depth,
            progress_truncated,
        );
        if !updated && !state_updated && !subchat_id.is_empty() {
            let mut card =
                ToolCard::from_tool_call(&json!({"id": tool_call_id, "name": "subagent"}));
            apply_subchat_update_to_card(
                &mut card,
                subchat_id,
                &progress,
                attached_files,
                depth,
                progress_truncated,
            );
            self.push_history_item(TranscriptItem::Tool(card));
            self.selected_tool_index = self.transcript.len().checked_sub(1);
        }
        if let Some(summary) = self
            .subagent_summaries()
            .first()
            .map(SubagentSummary::detail)
        {
            self.set_working_detail(summary);
        }
    }

    pub(super) fn update_visible_subchat(
        &mut self,
        tool_call_id: &str,
        subchat_id: &str,
        progress: &str,
        attached_files: &[String],
        depth: usize,
        progress_truncated: bool,
    ) -> bool {
        for (idx, item) in self.transcript.iter_mut().enumerate().rev() {
            let TranscriptItem::Tool(card) = item else {
                continue;
            };
            if card.id != tool_call_id {
                continue;
            }
            apply_subchat_update_to_card(
                card,
                subchat_id,
                progress,
                attached_files,
                depth,
                progress_truncated,
            );
            self.selected_tool_index = Some(idx);
            return true;
        }
        false
    }

    pub(super) fn update_state_subchat(
        &mut self,
        tool_call_id: &str,
        subchat_id: &str,
        progress: &str,
        attached_files: &[String],
        depth: usize,
        progress_truncated: bool,
    ) -> bool {
        let mut updated = false;
        for message in self.transcript_state.messages_mut() {
            if message.role.is_tool_result()
                && message.tool_call_id.as_deref() == Some(tool_call_id)
            {
                updated = true;
            }
            if message.role != TranscriptRole::Assistant {
                continue;
            }
            for tool in &mut message.tool_calls {
                if tool
                    .get("id")
                    .or_else(|| tool.get("tool_call_id"))
                    .and_then(Value::as_str)
                    != Some(tool_call_id)
                {
                    continue;
                }
                updated = true;
                apply_subchat_update_to_tool_value(
                    tool,
                    subchat_id,
                    progress,
                    attached_files,
                    depth,
                    progress_truncated,
                );
            }
        }
        updated
    }

    pub(super) fn complete_tool(
        &mut self,
        id: &str,
        name: &str,
        result: String,
        status: ToolStatus,
    ) {
        let active_ask_tool_id = self
            .ask_questions_form
            .as_ref()
            .map(|form| form.tool_call_id().to_string());
        for (idx, item) in self.transcript.iter_mut().enumerate().rev() {
            if let TranscriptItem::Tool(card) = item {
                if card.id == id || id.is_empty() {
                    card.set_result(&result);
                    card.status = status;
                    card.subchat_active = false;
                    if active_ask_tool_id.as_deref() == Some(card.id.as_str()) {
                        card.expanded = false;
                    }
                    let detail = card.summary();
                    self.selected_tool_index = Some(idx);
                    if self.session_state.shows_working_indicator() && !detail.is_empty() {
                        self.working_detail = Some(detail);
                    }
                    self.finalize_matching_tool_messages(id);
                    return;
                }
            }
        }
        let mut card = ToolCard::from_tool_call(&json!({"id": id, "name": name}));
        card.set_result(&result);
        card.status = status;
        if active_ask_tool_id.as_deref() == Some(card.id.as_str()) {
            card.expanded = false;
        }
        let item = TranscriptItem::Tool(card);
        self.push_live_item(item);
        self.selected_tool_index = Some(self.transcript.len() - 1);
        self.finalize_matching_tool_messages(id);
    }

    pub(super) fn finalize_tool_cards_for_turn(&mut self) {
        for item in &mut self.transcript {
            let TranscriptItem::Tool(card) = item else {
                continue;
            };
            if card.subchat_active {
                card.subchat_active = false;
                if card.status.is_active() {
                    card.status = ToolStatus::Succeeded;
                }
            }
        }
        if self.native_scrollback {
            self.move_completed_tool_cards_to_history();
        }
    }

    pub(super) fn move_completed_tool_cards_to_history(&mut self) {
        let mut idx = 0usize;
        while idx < self.transcript.len() {
            let completed = matches!(
                self.transcript.get(idx),
                Some(TranscriptItem::Tool(card)) if card.status.is_final()
            );
            if !completed {
                idx += 1;
                continue;
            }
            let item = self.transcript.remove(idx);
            self.history.enqueue(item);
            self.selected_tool_index = self.selected_tool_index.and_then(|selected| {
                if selected == idx {
                    None
                } else if selected > idx {
                    Some(selected - 1)
                } else {
                    Some(selected)
                }
            });
        }
    }

    pub(super) fn finalize_matching_tool_messages(&mut self, id: &str) {
        for message in self.transcript_state.messages_mut() {
            if message.role.is_tool_result()
                && (message.tool_call_id.as_deref() == Some(id) || id.is_empty())
            {
                message.stream_finished = true;
            }
        }
    }

    pub(super) fn toggle_selected_tool(&mut self) -> bool {
        let Some(index) = self.selected_tool_index else {
            return false;
        };
        if let Some(TranscriptItem::Tool(card)) = self.transcript.get_mut(index) {
            card.toggle();
            true
        } else {
            false
        }
    }

    pub(super) fn cycle_tool_selection(&mut self) {
        let indices = self
            .transcript
            .iter()
            .enumerate()
            .filter_map(|(idx, item)| matches!(item, TranscriptItem::Tool(_)).then_some(idx))
            .collect::<Vec<_>>();
        if indices.is_empty() {
            self.selected_tool_index = None;
            return;
        }
        let next = match self.selected_tool_index {
            Some(current) => indices
                .iter()
                .position(|idx| *idx == current)
                .map(|pos| indices[(pos + 1) % indices.len()])
                .unwrap_or(indices[0]),
            None => indices[0],
        };
        self.selected_tool_index = Some(next);
    }
}

impl App {
    pub(super) fn handle_snapshot(&mut self, raw: &Value) -> AppAction {
        if let Some(thread) = raw.get("thread") {
            self.worktree_meta = parse_worktree_meta(thread.get("worktree"), self);
            if let Some(title) = thread
                .get("title")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
            {
                self.session_title = Some(title.to_string());
            }
            self.permission_policy = session::PermissionPolicy {
                auto_approve_editing_tools: thread
                    .get("auto_approve_editing_tools")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                auto_approve_dangerous_commands: thread
                    .get("auto_approve_dangerous_commands")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            };
            if let Some(model) = thread.get("model") {
                if let Some(model) = model.as_str().filter(|value| !value.is_empty()) {
                    self.model = Some(model.to_string());
                }
            }
            if let Some(mode) = thread.get("mode") {
                if let Some(mode) = mode.as_str().filter(|value| !value.is_empty()) {
                    self.mode = Some(mode.to_string());
                }
            }
            self.task_id = task_id_from_thread(thread);
            self.boost_reasoning = thread
                .get("boost_reasoning")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            self.reasoning_effort = thread
                .get("reasoning_effort")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            self.thread_params = thread.clone();
            self.refresh_settings_surface();
        }
        if let Some(messages) = raw.get("messages").and_then(Value::as_array) {
            self.transcript_state.reset_from_messages(messages);
            let include_header = self.show_session_header || self.session_title.is_some();
            if self.native_scrollback {
                let messages = self.transcript_state.messages().to_vec();
                let mut next_keys = Vec::new();
                if include_header {
                    next_keys.push(session_header_key(
                        &self.session_header_title(),
                        &self.session_header_subtitle(),
                    ));
                }
                for message in &messages {
                    next_keys.extend(rendered_state_keys_for_message(message));
                }
                if !next_keys.is_empty() && self.rendered_state_keys == next_keys {
                    self.rendered_state_cursor = self.rendered_state_keys.len();
                } else {
                    self.replace_live_region_from_snapshot(&next_keys);
                    if include_header && self.rendered_state_cursor == 0 {
                        self.push_session_header();
                    }
                    for message in &messages {
                        self.append_render_message(message);
                    }
                    self.rendered_state_keys
                        .truncate(self.rendered_state_cursor);
                }
            } else {
                self.rebuild_render_transcript_from_state();
                if include_header {
                    self.transcript.insert(0, self.session_header_item());
                }
            }
        }
        if let Some(runtime) = raw.get("runtime") {
            let became_idle = self.apply_runtime_state(runtime);
            self.maybe_open_pending_ask_questions_form();
            self.update_usage(runtime);
            self.update_server_queue_from_runtime(runtime);
            self.sync_runtime_approvals(runtime);
            if became_idle {
                return self.dispatch_next_queued_input();
            }
        }
        AppAction::None
    }
}

fn task_id_from_thread(thread: &Value) -> Option<String> {
    thread
        .get("task_meta")
        .and_then(|meta| meta.get("task_id"))
        .and_then(Value::as_str)
        .filter(|task_id| !task_id.is_empty())
        .map(str::to_string)
}

impl App {
    fn update_worktree_meta(&mut self, raw: &Value) {
        if raw.get("worktree").is_none() {
            return;
        }
        self.worktree_meta = parse_worktree_meta(raw.get("worktree"), self);
    }
}

fn parse_worktree_meta(value: Option<&Value>, app: &mut App) -> Option<WorktreeMeta> {
    match value {
        Some(Value::Null) | None => None,
        Some(value) => match serde_json::from_value::<WorktreeMeta>(value.clone()) {
            Ok(meta) => Some(meta),
            Err(error) => {
                app.add_notice(format!("Ignored invalid worktree metadata: {error}"));
                None
            }
        },
    }
}

impl App {
    pub(super) fn handle_message_added_payload(
        &mut self,
        message: Option<&Value>,
        index: Option<usize>,
    ) {
        let Some(raw_message) = message else {
            return;
        };
        let message = TranscriptMessage::from_wire(raw_message);
        if message.role == TranscriptRole::User {
            let client_message_id = message.client_message_id().map(str::to_string);
            let message_count = self.transcript_state.messages().len();
            let out_of_range_index = index.filter(|index| *index > message_count);
            if self
                .transcript_state
                .replace_optimistic_user_message_at(message.clone(), index)
            {
                if let Some(client_message_id) = client_message_id {
                    if let Some(in_flight) = self.in_flight_send.as_mut().filter(|in_flight| {
                        in_flight.correlation.client_message_id == client_message_id
                    }) {
                        in_flight.accepted = true;
                    }
                }
                self.rebuild_remote_transcript_from_state();
                if self.native_scrollback && self.history.inserted_cell_count() > 0 {
                    self.resize_reflow.schedule_immediate();
                }
                if let Some(index) = out_of_range_index {
                    self.add_notice(format!(
                        "Server message index {index} exceeds transcript length {message_count}; appended"
                    ));
                }
                return;
            }
            if message.client_message_id().is_none()
                && self.transcript_state.has_optimistic_user_message()
            {
                tracing::warn!(
                    "server user-message echo has no client_message_id; optimistic message cannot be reconciled"
                );
            }
        }
        let state_keys = rendered_state_keys_for_message(&message);
        let replayed = !state_keys.is_empty()
            && state_keys.into_iter().all(|key| {
                state_key_has_stable_identity(&key)
                    && self
                        .rendered_state_keys
                        .iter()
                        .any(|existing| existing == &key)
            });
        let server_message_index = message.message_id.as_deref().and_then(|message_id| {
            self.transcript_state
                .messages()
                .iter()
                .position(|existing| existing.message_id.as_deref() == Some(message_id))
        });
        if replayed && (index.is_none() || server_message_index == index) {
            return;
        }
        let replaces_server_message = server_message_index.is_some();
        if !replaces_server_message
            && message.role.is_tool_result()
            && self.replace_state_tool_message(&message)
        {
            self.rebuild_remote_transcript_from_state();
            return;
        }
        let message_count = self.transcript_state.messages().len();
        let insert_before_end = index.is_some_and(|index| index < message_count);
        let out_of_range_index = index.filter(|index| *index > message_count);
        let added = self.transcript_state.add_message_at(raw_message, index);
        if !added {
            self.rebuild_remote_transcript_from_state();
            if self.native_scrollback && self.history.inserted_cell_count() > 0 {
                self.resize_reflow.schedule_immediate();
            }
            if let Some(index) = out_of_range_index {
                self.add_notice(format!(
                    "Server message index {index} exceeds transcript length {message_count}; appended"
                ));
            }
            return;
        }
        if insert_before_end
            || message.role == TranscriptRole::Plan
            || is_plan_delta_message(&message)
        {
            self.rebuild_remote_transcript_from_state();
        } else {
            match &message.role {
                TranscriptRole::Tool | TranscriptRole::Diff => {
                    self.push_state_tool_result(&message)
                }
                TranscriptRole::Assistant
                | TranscriptRole::User
                | TranscriptRole::ClientLocalNotice
                | TranscriptRole::Plan
                | TranscriptRole::Goal
                | TranscriptRole::Event
                | TranscriptRole::System
                | TranscriptRole::ContextFile
                | TranscriptRole::PlainText
                | TranscriptRole::CdInstruction
                | TranscriptRole::CompressionReport
                | TranscriptRole::Error
                | TranscriptRole::Unknown { .. } => self.append_render_message(&message),
            }
        }
        if let Some(index) = out_of_range_index {
            self.add_notice(format!(
                "Server message index {index} exceeds transcript length {message_count}; appended"
            ));
        }
    }

    pub(super) fn replace_state_tool_message(&mut self, message: &TranscriptMessage) -> bool {
        let Some(tool_call_id) = message
            .tool_call_id
            .as_deref()
            .filter(|tool_call_id| !tool_call_id.is_empty())
        else {
            return false;
        };
        let Some(existing) = self
            .transcript_state
            .messages_mut()
            .iter_mut()
            .find(|existing| {
                existing.role.is_tool_result()
                    && existing.tool_call_id.as_deref() == Some(tool_call_id)
            })
        else {
            return false;
        };
        *existing = message.clone();
        true
    }

    pub(super) fn handle_message_updated_payload(
        &mut self,
        message_id: Option<&str>,
        message: Option<&Value>,
    ) {
        let Some(message) = message else {
            return;
        };
        self.transcript_state.update_message(message_id, message);
        self.rebuild_remote_transcript_from_state();
    }

    pub(super) fn handle_message_removed(&mut self, message_id: Option<&str>) {
        if self.transcript_state.remove_message(message_id) {
            self.rebuild_remote_transcript_from_state();
        }
    }

    pub(super) fn handle_messages_truncated(&mut self, from_index: usize) {
        self.transcript_state.truncate_messages(from_index);
        self.rebuild_remote_transcript_from_state();
    }
}

impl App {
    pub(super) fn push_state_tool_result(&mut self, message: &TranscriptMessage) {
        let key = render_message_key(message, "tool", 0);
        self.maybe_open_ask_questions_form(message);
        if !self.record_state_history_key(key) {
            return;
        }
        let tool_call_id = message.tool_call_id.as_deref().unwrap_or_default();
        self.complete_tool(
            tool_call_id,
            message.role.as_str(),
            message.content.clone(),
            if message.tool_failed {
                ToolStatus::Failed
            } else {
                ToolStatus::Succeeded
            },
        );
        if let Some(TranscriptItem::Tool(card)) = self
            .transcript
            .iter_mut()
            .rev()
            .find(|item| {
                matches!(item, TranscriptItem::Tool(card) if card.id == tool_call_id || tool_call_id.is_empty())
            })
        {
            card.apply_result_metadata(&message.extra);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SubagentSummary {
    pub(super) tool_call_id: String,
    pub(super) tool_name: String,
    pub(super) progress: Option<String>,
    pub(super) attached_files: usize,
    pub(super) depth: usize,
    pub(super) active: bool,
    pub(super) truncated: bool,
}

impl SubagentSummary {
    pub(super) fn detail(&self) -> String {
        let mut parts = vec![format!(
            "{} [{}]",
            tool_display_name(&self.tool_name),
            sanitize_tool_inline(&self.tool_call_id)
        )];
        parts.push(if self.active { "active" } else { "recent" }.to_string());
        if self.depth > 1 {
            parts.push(format!("depth {}", self.depth));
        }
        if self.attached_files > 0 {
            parts.push(format!("{} files", self.attached_files));
        }
        if self.truncated {
            parts.push("truncated".to_string());
        }
        if let Some(progress) = &self.progress {
            parts.push(sanitize_tool_text(progress));
        }
        parts.join(" · ")
    }
}

fn truncate_subchat_progress(progress: &str) -> (String, bool) {
    truncate_graphemes(&sanitize_tool_text(progress), MAX_SUBCHAT_PROGRESS_CHARS)
}

fn apply_subchat_update_to_card(
    card: &mut ToolCard,
    subchat_id: &str,
    progress: &str,
    attached_files: &[String],
    depth: usize,
    progress_truncated: bool,
) {
    if subchat_id.is_empty() {
        card.clear_subchat();
        return;
    }
    card.subchat_active = true;
    card.subchat_depth = depth.clamp(1, MAX_SUBCHAT_DEPTH);
    card.subchat_updates = card.subchat_updates.saturating_add(1);
    if progress_truncated {
        card.subchat_truncated = true;
    }
    if !subchat_id.contains("/tool:") && !progress.is_empty() {
        card.subchat_log.clear();
        card.subchat_log.push(progress.to_string());
    }
    for file in attached_files {
        let file = sanitize_tool_text(file);
        if file.is_empty() || card.attached_files.contains(&file) {
            continue;
        }
        if card.attached_files.len() < MAX_SUBCHAT_ATTACHED_FILES {
            card.attached_files.push(file);
        } else {
            card.subchat_truncated = true;
        }
    }
}

fn apply_subchat_update_to_tool_value(
    tool: &mut Value,
    subchat_id: &str,
    progress: &str,
    attached_files: &[String],
    depth: usize,
    progress_truncated: bool,
) {
    let Value::Object(map) = tool else {
        return;
    };
    if subchat_id.is_empty() {
        map.remove("subchat");
        map.insert("subchat_log".to_string(), Value::Array(Vec::new()));
        map.insert("attached_files".to_string(), Value::Array(Vec::new()));
        map.insert("subchat_updates".to_string(), json!(0));
        map.insert("subchat_depth".to_string(), json!(1));
        map.insert("subchat_truncated".to_string(), Value::Bool(false));
        return;
    }
    map.insert("subchat".to_string(), Value::String(subchat_id.to_string()));
    map.insert(
        "subchat_depth".to_string(),
        Value::Number((depth.clamp(1, MAX_SUBCHAT_DEPTH) as u64).into()),
    );
    let updates = map
        .get("subchat_updates")
        .and_then(Value::as_u64)
        .unwrap_or_default()
        .saturating_add(1);
    map.insert("subchat_updates".to_string(), Value::Number(updates.into()));
    let mut truncated = map
        .get("subchat_truncated")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || progress_truncated;
    if !subchat_id.contains("/tool:") && !progress.is_empty() {
        map.insert(
            "subchat_log".to_string(),
            Value::Array(vec![Value::String(progress.to_string())]),
        );
    }
    let mut files = map
        .get("attached_files")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|value| value.as_str().map(str::to_string))
        .collect::<Vec<_>>();
    for file in attached_files {
        let file = sanitize_tool_text(file);
        if file.is_empty() || files.contains(&file) {
            continue;
        }
        if files.len() < MAX_SUBCHAT_ATTACHED_FILES {
            files.push(file);
        } else {
            truncated = true;
        }
    }
    map.insert(
        "attached_files".to_string(),
        Value::Array(files.into_iter().map(Value::String).collect()),
    );
    map.insert("subchat_truncated".to_string(), Value::Bool(truncated));
}

fn stable_scope_hash(values: &[String]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for value in values {
        for byte in value.as_bytes().iter().copied().chain(std::iter::once(0)) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    hash
}

fn approval_reasons_present(raw: &Value) -> bool {
    raw.get("reasons").is_some() || raw.get("pause_reasons").is_some()
}

fn approval_tool_call_ids(raw: &Value) -> Vec<String> {
    let mut ids = Vec::new();
    collect_tool_call_ids(raw, &mut ids);
    for key in ["tool_call_ids", "tool_ids"] {
        if let Some(value) = raw.get(key) {
            collect_tool_call_ids(value, &mut ids);
        }
    }
    for key in ["reasons", "pause_reasons", "decisions"] {
        if let Some(values) = raw.get(key).and_then(Value::as_array) {
            for value in values {
                collect_tool_call_ids(value, &mut ids);
            }
        }
    }
    ids.sort();
    ids.dedup();
    ids
}

fn collect_tool_call_ids(value: &Value, ids: &mut Vec<String>) {
    match value {
        Value::String(id) if !id.is_empty() => ids.push(id.clone()),
        Value::Object(map) => {
            if let Some(id) = map
                .get("tool_call_id")
                .or_else(|| map.get("tool_id"))
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
            {
                ids.push(id.to_string());
            }
            for key in ["tool_call_ids", "tool_ids"] {
                if let Some(value) = map.get(key) {
                    collect_tool_call_ids(value, ids);
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_tool_call_ids(value, ids);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn project() -> OpenProjectResponse {
        OpenProjectResponse {
            project_id: "p1".to_string(),
            slug: "demo".to_string(),
            root: PathBuf::from("/tmp/demo"),
            pinned: Some(false),
            worker: None,
            cron_pending: None,
        }
    }

    #[test]
    fn approval_tool_call_ids_deduplicate_nested_values() {
        let ids = approval_tool_call_ids(&json!({
            "reasons": [{"tool_call_id": "call-2"}],
            "tool_call_ids": ["call-1", "call-2"],
        }));

        assert_eq!(ids, ["call-1", "call-2"]);
    }

    #[test]
    fn usage_updates_clear_explicit_empty_and_null_values() {
        let mut app = App::new(project());

        app.update_usage_value(&json!({
            "prompt_tokens": 12,
            "completion_tokens": 8,
        }));
        assert_eq!(app.usage().unwrap().total_tokens, Some(20));

        app.update_usage_value(&Value::Null);
        assert_eq!(app.usage(), None);

        app.update_usage_value(&json!({
            "prompt_tokens": 12,
            "completion_tokens": 8,
        }));
        app.update_usage_value(&json!({}));
        assert_eq!(app.usage(), None);

        app.update_usage_value(&json!({
            "prompt_tokens": 12,
            "completion_tokens": 8,
        }));
        app.update_usage_value(&json!({"prompt_tokens": 9}));
        assert_eq!(app.usage().unwrap().prompt_tokens, Some(9));
        assert_eq!(app.usage().unwrap().completion_tokens, None);
        assert_eq!(app.usage().unwrap().total_tokens, None);
    }
}
