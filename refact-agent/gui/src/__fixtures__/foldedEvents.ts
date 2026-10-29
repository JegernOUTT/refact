import type {
  AssistantMessage,
  EventMessage,
  ToolMessage,
} from "../services/refact";

export function toolCallMessage(
  id: string,
  name: string,
  args: Record<string, unknown> = {},
): AssistantMessage {
  return {
    role: "assistant",
    content: "",
    message_id: `assistant-${id}`,
    tool_calls: [
      {
        id,
        index: 0,
        type: "function",
        function: { name, arguments: JSON.stringify(args) },
      },
    ],
  };
}

export function execResult(
  toolCallId: string,
  processId: string,
  status = "running",
): ToolMessage {
  return {
    role: "tool",
    tool_call_id: toolCallId,
    content: "Process started",
    extra: {
      exec: {
        process_id: processId,
        status,
        mode: "background",
        short_description: "dev server",
      },
    },
  };
}

export function agentResult(toolCallId: string, agentId: string): ToolMessage {
  return {
    role: "tool",
    tool_call_id: toolCallId,
    content: "Started background agent",
    background_agent_id: agentId,
  };
}

export function processCompleted(
  processId: string,
  messageId = `event-${processId}`,
): EventMessage {
  const payload = {
    process_id: processId,
    status: "exited",
    exit_code: 0,
    duration_ms: 1500,
    short_description: "dev server",
    mode: "background",
  };
  return {
    role: "event",
    content: `Background process '${processId}' exited (exit 0)`,
    message_id: messageId,
    subkind: "process_completed",
    source: "exec.registry",
    payload,
    extra: {
      event: { subkind: "process_completed", source: "exec.registry", payload },
    },
  };
}

export function agentFinished(
  agentId: string,
  messageId = `event-${agentId}`,
): EventMessage {
  const payload = {
    agent_id: agentId,
    status: "completed",
    title: "Investigate flaky test",
    child_chat_id: `child-${agentId}`,
    tokens_used: 1200,
  };
  return {
    role: "event",
    content:
      '[background subagent finished]\nagent_id: x\n\nSummary:\nAll good, fixed it.\n\n(call agent_result(agent_id="x") for details)',
    message_id: messageId,
    subkind: "system_notice",
    source: "agents.spawn",
    payload,
    extra: {
      event: { subkind: "system_notice", source: "agents.spawn", payload },
    },
  };
}

export function storedProcessStartResult(
  toolCallId: string,
  processId: string,
): ToolMessage {
  return {
    message_id: `tool-${toolCallId}`,
    role: "tool",
    content: [
      "Process started",
      `process_id: ${processId}`,
      "short_description: Start local GUI preview for the Buddy page audit",
      "status: failed",
      "mode: background",
      "service_name: <none>",
      "command: npm run dev -- --host 127.0.0.1 --port 5173 --strictPort",
      "cwd: /workspace/refact-agent/gui",
      "started_at: Some(1792930874877)",
      "exit_code: <none>",
      "failure_reason: startup readiness timed out after 1.000s",
      "",
      "stdout:",
      "<empty>",
      "",
      "stderr:",
      "<empty>",
    ].join("\n"),
    tool_call_id: toolCallId,
    tool_failed: true,
  };
}

function storedEvent(raw: Record<string, unknown>): EventMessage {
  return raw as EventMessage;
}

export function storedProcessCompleted(processId: string): EventMessage {
  return storedEvent({
    message_id: `event-${processId}`,
    role: "event",
    content:
      "Background process 'Start local GUI preview for the Buddy page audit' failed (exit unknown)",
    event: {
      subkind: "process_completed",
      source: "exec.registry",
      payload: {
        process_id: processId,
        status: "failed",
        exit_code: null,
        duration_ms: 1281,
        short_description: "Start local GUI preview for the Buddy page audit",
        mode: "background",
        push: "append",
      },
    },
  });
}

export function storedSubagentLaunchResult(
  toolCallId: string,
  agentId: string,
): ToolMessage {
  return {
    message_id: `tool-${toolCallId}`,
    role: "tool",
    content: `✓ Accepted background subagent: Read-only reconnaissance\n- agent_id: ${agentId}\n- status: queued\n- child_chat_id: subchat-1\n\nThe completion will be pushed back into this chat automatically.`,
    tool_call_id: toolCallId,
    tool_failed: false,
  };
}

export function storedSubagentFinished(
  parentToolCallId: string,
  agentId: string,
  status = "completed",
): EventMessage {
  return storedEvent({
    message_id: `event-${agentId}`,
    role: "event",
    content: `[background subagent finished]\nagent_id: ${agentId}\nstatus: ${status}\ntitle: Subagent: Read-only reconnaissance\n\nSummary:\nThe page renders fine.\n\n(call agent_result(agent_id="${agentId}", include_details=true) for stored payload/details, or open the child trajectory)`,
    event: {
      subkind: "system_notice",
      source: "agents.spawn",
      payload: {
        agent_id: agentId,
        parent_tool_call_id: parentToolCallId,
        kind: "subagent",
        status,
        title: "Subagent: Read-only reconnaissance",
        child_chat_id: "subchat-1",
        tokens_used: 2355037,
      },
    },
  });
}
