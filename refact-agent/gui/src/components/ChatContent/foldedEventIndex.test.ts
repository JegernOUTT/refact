import { describe, expect, it } from "vitest";
import type {
  ChatMessages,
  EventMessage,
  ToolMessage,
} from "../../services/refact";
import {
  agentFinished,
  agentResult,
  execResult,
  processCompleted,
  storedProcessCompleted,
  storedProcessStartResult,
  storedSubagentFinished,
  storedSubagentLaunchResult,
  toolCallMessage,
} from "../../__fixtures__/foldedEvents";
import { chatReducer } from "../../features/Chat/Thread/reducer";
import {
  applyChatEvent,
  newChatAction,
} from "../../features/Chat/Thread/actions";
import type { ChatEventEnvelope } from "../../services/refact/chatSubscription";
import { buildFoldedEventIndex, foldedEventIndexFor } from "./foldedEventIndex";

describe("buildFoldedEventIndex", () => {
  it("folds a process_completed event into the process_start row that started it", () => {
    const messages: ChatMessages = [
      toolCallMessage("call-start", "process_start", {
        command: "npm run dev",
      }),
      execResult("call-start", "proc-1"),
      processCompleted("proc-1"),
    ];

    const index = buildFoldedEventIndex(messages);

    expect([...index.foldedMessageIndices]).toEqual([2]);
    expect(index.processEventByToolCallId.get("call-start")?.message_id).toBe(
      "event-proc-1",
    );
  });

  it("matches shell, t_ aliases and shell_service start actions", () => {
    const messages: ChatMessages = [
      toolCallMessage("call-shell", "t_shell", { command: "sleep 1" }),
      execResult("call-shell", "proc-shell"),
      toolCallMessage("call-service", "shell_service", {
        action: "start",
        service_name: "api",
      }),
      execResult("call-service", "proc-service"),
      processCompleted("proc-shell"),
      processCompleted("proc-service"),
    ];

    const index = buildFoldedEventIndex(messages);

    expect([...index.foldedMessageIndices].sort()).toEqual([4, 5]);
    expect(index.processEventByToolCallId.has("call-shell")).toBe(true);
    expect(index.processEventByToolCallId.has("call-service")).toBe(true);
  });

  it("never lets reads, waits, kills, stdin writes or service status absorb the event", () => {
    const messages: ChatMessages = [
      toolCallMessage("call-read", "process_read", { process_id: "proc-1" }),
      execResult("call-read", "proc-1"),
      toolCallMessage("call-wait", "process_wait", { process_id: "proc-1" }),
      execResult("call-wait", "proc-1", "exited"),
      toolCallMessage("call-kill", "process_kill", { process_id: "proc-1" }),
      execResult("call-kill", "proc-1", "killed"),
      toolCallMessage("call-stdin", "process_write_stdin", {
        process_id: "proc-1",
      }),
      execResult("call-stdin", "proc-1"),
      toolCallMessage("call-status", "shell_service", {
        action: "status",
        service_name: "api",
      }),
      execResult("call-status", "proc-1"),
      processCompleted("proc-1"),
    ];

    const index = buildFoldedEventIndex(messages);

    expect(index.foldedMessageIndices.size).toBe(0);
    expect(index.processEventByToolCallId.size).toBe(0);
  });

  it("keeps events whose start row is not in the thread", () => {
    const messages: ChatMessages = [
      toolCallMessage("call-start", "process_start"),
      execResult("call-start", "proc-other"),
      processCompleted("proc-1"),
    ];

    expect(buildFoldedEventIndex(messages).foldedMessageIndices.size).toBe(0);
  });

  it("folds a subagent finished event into the launch row with the same agent id", () => {
    const messages: ChatMessages = [
      toolCallMessage("call-agent", "subagent", { task: "Investigate" }),
      agentResult("call-agent", "agent-1"),
      agentFinished("agent-1"),
      agentFinished("agent-unknown"),
    ];

    const index = buildFoldedEventIndex(messages);

    expect([...index.foldedMessageIndices]).toEqual([2]);
    expect(index.agentEventByToolCallId.get("call-agent")?.message_id).toBe(
      "event-agent-1",
    );
  });

  it("matches delegate launches and agent ids carried in result extra", () => {
    const launch: ToolMessage = {
      role: "tool",
      tool_call_id: "call-delegate",
      content: "Delegated",
      extra: { background_agent_id: "agent-2" },
    };
    const messages: ChatMessages = [
      toolCallMessage("call-delegate", "delegate", { task: "Refactor" }),
      launch,
      agentFinished("agent-2"),
    ];

    expect([...buildFoldedEventIndex(messages).foldedMessageIndices]).toEqual([
      2,
    ]);
  });

  it("keeps finished events whose status the launch row cannot show", () => {
    const closed = agentFinished("agent-1");
    const payload = { agent_id: "agent-1", status: "closed" };
    const messages: ChatMessages = [
      toolCallMessage("call-agent", "subagent"),
      agentResult("call-agent", "agent-1"),
      {
        ...closed,
        payload,
        extra: {
          event: { subkind: "system_notice", source: "agents.spawn", payload },
        },
      },
    ];

    expect(buildFoldedEventIndex(messages).foldedMessageIndices.size).toBe(0);
  });

  it("does not fold the spawn brief event that has no status", () => {
    const brief: EventMessage = {
      role: "event",
      content: "Do the task",
      subkind: "system_notice",
      source: "agents.spawn",
      payload: { config_name: "default", agent_id: "agent-1" },
    };
    const messages: ChatMessages = [
      toolCallMessage("call-agent", "subagent"),
      agentResult("call-agent", "agent-1"),
      brief,
    ];

    expect(buildFoldedEventIndex(messages).foldedMessageIndices.size).toBe(0);
  });

  it("folds a stored process_start result that carries the process id only in its content header", () => {
    const messages: ChatMessages = [
      toolCallMessage("call_811546", "process_start", {
        command: "npm run dev",
      }),
      storedProcessStartResult("call_811546", "exec_7e7788ba"),
      storedProcessCompleted("exec_7e7788ba"),
    ];

    const index = buildFoldedEventIndex(messages);

    expect([...index.foldedMessageIndices]).toEqual([2]);
    expect(index.processEventByToolCallId.get("call_811546")?.subkind).toBe(
      "process_completed",
    );
  });

  it("folds a stored subagent finished event by parent_tool_call_id when the launch result has no agent id", () => {
    const messages: ChatMessages = [
      toolCallMessage("call_f2c0e2", "subagent", { task: "Recon" }),
      storedSubagentLaunchResult("call_f2c0e2", "bgagent-b54f"),
      storedSubagentFinished("call_f2c0e2", "bgagent-b54f", "failed"),
    ];

    const index = buildFoldedEventIndex(messages);

    expect([...index.foldedMessageIndices]).toEqual([2]);
    expect(index.agentEventByToolCallId.get("call_f2c0e2")?.message_id).toBe(
      "event-bgagent-b54f",
    );
  });

  it("ignores a parent_tool_call_id that is not a subagent launch in this thread", () => {
    const messages: ChatMessages = [
      toolCallMessage("call-shell", "shell", { command: "ls" }),
      storedSubagentFinished("call-shell", "bgagent-x"),
      storedSubagentFinished("call-elsewhere", "bgagent-y"),
    ];

    expect(buildFoldedEventIndex(messages).foldedMessageIndices.size).toBe(0);
  });

  it("does not let a compressed assistant absorb events", () => {
    const compressed = {
      ...toolCallMessage("call-old", "process_start"),
      extra: {
        compression: {
          kind: "llm_segment_summary",
          source_hash: "h1",
          source_message_ids: ["m1"],
        },
      },
    };
    const messages: ChatMessages = [
      compressed,
      storedProcessStartResult("call-old", "exec_old"),
      storedProcessCompleted("exec_old"),
    ];

    expect(buildFoldedEventIndex(messages).foldedMessageIndices.size).toBe(0);
  });

  it("folds stored messages delivered through the real snapshot reducer path", () => {
    const empty = chatReducer(undefined, { type: "@@INIT" });
    const initial = chatReducer(empty, newChatAction(undefined));
    const chatId = initial.current_thread_id;
    const snapshot: ChatEventEnvelope = {
      chat_id: chatId,
      seq: "1",
      type: "snapshot",
      thread: {
        id: chatId,
        title: "audit",
        model: "gpt-4",
        mode: "AGENT",
        tool_use: "agent",
        boost_reasoning: false,
        context_tokens_cap: 8192,
        include_project_info: true,
        checkpoints_enabled: true,
        is_title_generated: true,
      },
      runtime: {
        state: "idle",
        paused: false,
        error: null,
        queue_size: 0,
        pause_reasons: [],
        queued_items: [],
      },
      background_agents: [],
      messages: [
        toolCallMessage("call_811546", "process_start", { command: "npm" }),
        storedProcessStartResult("call_811546", "exec_7e7788ba"),
        toolCallMessage("call_f2c0e2", "subagent", { task: "Recon" }),
        storedSubagentLaunchResult("call_f2c0e2", "bgagent-b54f"),
        storedProcessCompleted("exec_7e7788ba"),
        storedSubagentFinished("call_f2c0e2", "bgagent-b54f"),
      ],
    };

    const state = chatReducer(initial, applyChatEvent(snapshot));
    const messages = state.threads[chatId]?.thread.messages ?? [];
    const index = foldedEventIndexFor(messages);

    expect(index.processEventByToolCallId.has("call_811546")).toBe(true);
    expect(index.agentEventByToolCallId.has("call_f2c0e2")).toBe(true);
    expect(index.foldedMessageIndices.size).toBe(2);
  });
});
