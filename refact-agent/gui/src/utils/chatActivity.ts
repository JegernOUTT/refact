import {
  Activity,
  Circle,
  CircleAlert,
  CircleCheck,
  Clock,
  Wrench,
  MessageCircleQuestion,
  Pause,
  Terminal,
  Users,
} from "lucide-react";
import type { ChatThreadRuntime } from "../features/Chat/Thread/types";
import type { ChatMessage, ToolCall } from "../services/refact/types";
import { normalizeToolName } from "./toolNameAliases";

export type ChatActivity =
  | "idle"
  | "working"
  | "running_tools"
  | "needs_approval"
  | "waiting_agents"
  | "waiting_terminal"
  | "waiting_timer"
  | "running_command"
  | "waiting_user"
  | "needs_attention"
  | "paused"
  | "completed"
  | "error";

export const CHAT_ACTIVITY = {
  idle: { label: "Idle", tone: "muted", icon: Circle, status: "idle" },
  working: {
    label: "Working",
    tone: "accent",
    icon: Activity,
    status: "in_progress",
  },
  running_tools: {
    label: "Running tools",
    tone: "accent",
    icon: Wrench,
    status: "in_progress",
  },
  needs_approval: {
    label: "Needs approval",
    tone: "warning",
    icon: CircleAlert,
    status: "needs_attention",
  },
  waiting_agents: {
    label: "Waiting for subagents",
    tone: "accent",
    icon: Users,
    status: "in_progress",
  },
  waiting_terminal: {
    label: "Waiting for terminal",
    tone: "accent",
    icon: Terminal,
    status: "in_progress",
  },
  waiting_timer: {
    label: "Waiting for timer",
    tone: "accent",
    icon: Clock,
    status: "in_progress",
  },
  running_command: {
    label: "Running command",
    tone: "accent",
    icon: Terminal,
    status: "in_progress",
  },
  waiting_user: {
    label: "Waiting for your answer",
    tone: "warning",
    icon: MessageCircleQuestion,
    status: "needs_attention",
  },
  needs_attention: {
    label: "Needs your attention",
    tone: "warning",
    icon: CircleAlert,
    status: "needs_attention",
  },
  paused: {
    label: "Paused",
    tone: "warning",
    icon: Pause,
    status: "needs_attention",
  },
  completed: {
    label: "Completed",
    tone: "success",
    icon: CircleCheck,
    status: "completed",
  },
  error: { label: "Error", tone: "danger", icon: CircleAlert, status: "error" },
} as const;

export function activityFromSessionState(state?: string | null): ChatActivity {
  switch (state) {
    case "generating":
      return "working";
    case "executing_tools":
      return "running_tools";
    case "waiting_user_input":
      return "waiting_user";
    case "waiting_ide":
      return "needs_attention";
    case "paused":
      return "paused";
    case "completed":
      return "completed";
    case "error":
      return "error";
    default:
      return "idle";
  }
}

function record(value: unknown): Record<string, unknown> | undefined {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : undefined;
}

function argumentsOf(call: ToolCall): Record<string, unknown> | undefined {
  try {
    return record(JSON.parse(call.function.arguments));
  } catch {
    return undefined;
  }
}

type ActivityToolCall =
  | (Omit<ToolCall, "function"> & { function?: ToolCall["function"] | null })
  | null;

function toolActivity(call: ActivityToolCall): ChatActivity {
  if (
    !call ||
    typeof call.id !== "string" ||
    !call.id ||
    typeof call.function?.name !== "string"
  )
    return "running_tools";
  const name = normalizeToolName(
    call.function.name.replace(
      /^(?:functions\.|mcp__refact__|mcp_refact_)/,
      "",
    ),
  );
  const args = argumentsOf({ ...call, function: call.function });
  if (name === "wait") {
    const target = record(args?.target);
    if (target?.kind === "agents" || target?.kind === "task_agents")
      return "waiting_agents";
    if (target?.kind === "process") return "waiting_terminal";
    if (target?.kind === "duration") return "waiting_timer";
  }
  if (name === "shell" && args && args.run_in_background !== true)
    return "running_command";
  if (name === "process_write_stdin" && args) return "waiting_terminal";
  if (name?.startsWith("cmdline_") && args) return "running_command";
  return "running_tools";
}

export function unresolvedActivityCalls(
  messages: ChatMessage[],
): ActivityToolCall[] {
  const resolved = new Set<string>();
  for (let i = messages.length - 1; i >= 0; i--) {
    const message = messages[i];
    if (message.role === "user") return [];
    if (message.role === "assistant") {
      const serverIds = new Set(
        message.server_executed_tools?.map(
          (call: ActivityToolCall) => call?.id,
        ),
      );
      return (message.tool_calls ?? []).filter(
        (call: ActivityToolCall) =>
          typeof call?.id !== "string" ||
          !call.id ||
          (!call.id.startsWith("srvtoolu_") &&
            !resolved.has(call.id) &&
            !serverIds.has(call.id)),
      );
    }
    if ("tool_call_id" in message && typeof message.tool_call_id === "string") {
      resolved.add(message.tool_call_id);
    }
  }
  return [];
}

export function getChatActivity(
  runtime?: ChatThreadRuntime,
  fallbackState?: string | null,
): ChatActivity {
  if (!runtime) return activityFromSessionState(fallbackState);
  const state = runtime.session_state;
  const coarse = activityFromSessionState(state);
  if (state && ["idle", "completed", "error"].includes(coarse)) return coarse;
  if (!state && runtime.error) return "error";
  if (runtime.confirmation.pause) {
    return runtime.confirmation.pause_reasons.length
      ? "needs_approval"
      : "paused";
  }
  if (state === "paused" || state === "waiting_ide") return coarse;
  if (runtime.thread.pending_questions?.some((set) => set.status === "pending"))
    return "waiting_user";
  if (state === "waiting_user_input") return "waiting_user";
  if (state === "generating" || (!state && runtime.streaming)) return "working";
  if (state !== "executing_tools" && !runtime.waiting_for_response)
    return coarse;
  const calls = unresolvedActivityCalls(runtime.thread.messages);
  if (!calls.length) return "running_tools";
  const activities = calls.map(toolActivity);
  return activities.every((activity) => activity === activities[0])
    ? activities[0]
    : "running_tools";
}
