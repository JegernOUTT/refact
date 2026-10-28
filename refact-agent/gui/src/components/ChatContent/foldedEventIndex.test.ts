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
  toolCallMessage,
} from "../../__fixtures__/foldedEvents";
import { buildFoldedEventIndex } from "./foldedEventIndex";

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
});
