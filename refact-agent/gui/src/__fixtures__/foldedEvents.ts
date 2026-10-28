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
