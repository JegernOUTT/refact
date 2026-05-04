import type { TrajectoryMeta, TrajectoryEvent } from "./trajectories";
import type { TaskMeta, TaskBoard } from "./tasks";
import type { BuddySnapshot, BuddySSEEvent } from "../../features/Buddy/types";

export type { TrajectoryMeta, TrajectoryEvent };

export type TaskEvent =
  | { type: "snapshot"; tasks: TaskMeta[] }
  | { type: "task_created"; task_id: string; meta: TaskMeta }
  | { type: "task_updated"; task_id: string; meta: TaskMeta }
  | { type: "task_deleted"; task_id: string }
  | { type: "board_changed"; task_id: string; rev: number; board: TaskBoard };

export type NotificationEvent =
  | {
      type: "task_done";
      chat_id: string;
      tool_call_id: string;
      summary: string;
      knowledge_path?: string;
    }
  | {
      type: "ask_questions";
      chat_id: string;
      tool_call_id: string;
      questions: {
        id: string;
        type: string;
        text: string;
        options?: string[];
      }[];
    };

export type SidebarLoadingSection =
  | "workspace"
  | "trajectories"
  | "tasks"
  | "buddy";

export type SidebarLoadingStatus = "started" | "ready" | "error";

export type BuddySnapshotPayload = BuddySnapshot | { enabled: false } | null;

export type SidebarEvent =
  | {
      category: "snapshot";
      trajectories: TrajectoryMeta[];
      tasks: TaskMeta[];
      workspace_roots?: string[];
      buddy?: BuddySnapshotPayload;
    }
  | {
      category: "loading_phase";
      section: SidebarLoadingSection;
      status: SidebarLoadingStatus;
      elapsed_ms?: number;
      error?: string;
    }
  | {
      category: "workspace_snapshot";
      workspace_roots: string[];
    }
  | {
      category: "trajectories_snapshot";
      trajectories: TrajectoryMeta[];
    }
  | {
      category: "tasks_snapshot";
      tasks: TaskMeta[];
    }
  | {
      category: "buddy_snapshot";
      buddy: BuddySnapshotPayload;
    }
  | ({ category: "trajectory" } & TrajectoryEvent)
  | ({ category: "task" } & TaskEvent)
  | ({ category: "notification" } & NotificationEvent)
  | { category: "buddy"; buddy_event: BuddySSEEvent };

export type SidebarEventEnvelope = {
  seq: number;
} & SidebarEvent;

export type SidebarSubscriptionCallbacks = {
  onEvent: (event: SidebarEventEnvelope) => void;
  onError: (error: Error) => void;
  onConnected?: () => void;
  onDisconnected?: () => void;
};

function isValidSnapshot(obj: Record<string, unknown>): boolean {
  return (
    Array.isArray(obj.trajectories) &&
    Array.isArray(obj.tasks) &&
    (obj.workspace_roots === undefined || Array.isArray(obj.workspace_roots))
  );
}

function isValidTrajectoryEvent(obj: Record<string, unknown>): boolean {
  return typeof obj.type === "string" && typeof obj.id === "string";
}

function isValidTaskEvent(obj: Record<string, unknown>): boolean {
  if (typeof obj.type !== "string") return false;
  if (obj.type === "snapshot") return Array.isArray(obj.tasks);
  if (obj.type === "task_deleted") return typeof obj.task_id === "string";
  if (obj.type === "board_changed")
    return typeof obj.task_id === "string" && obj.board !== undefined;
  return typeof obj.task_id === "string" && obj.meta !== undefined;
}

function isValidNotificationEvent(obj: Record<string, unknown>): boolean {
  if (typeof obj.type !== "string") return false;
  if (typeof obj.chat_id !== "string") return false;
  if (typeof obj.tool_call_id !== "string") return false;

  if (obj.type === "task_done") {
    return typeof obj.summary === "string";
  }

  if (obj.type === "ask_questions") {
    return Array.isArray(obj.questions);
  }

  return false;
}

function isValidLoadingPhase(obj: Record<string, unknown>): boolean {
  const validSections = new Set<SidebarLoadingSection>([
    "workspace",
    "trajectories",
    "tasks",
    "buddy",
  ]);
  const validStatuses = new Set<SidebarLoadingStatus>([
    "started",
    "ready",
    "error",
  ]);
  if (
    typeof obj.section !== "string" ||
    !validSections.has(obj.section as SidebarLoadingSection)
  ) {
    return false;
  }
  if (
    typeof obj.status !== "string" ||
    !validStatuses.has(obj.status as SidebarLoadingStatus)
  ) {
    return false;
  }
  if (obj.elapsed_ms !== undefined && typeof obj.elapsed_ms !== "number") {
    return false;
  }
  if (obj.error !== undefined && typeof obj.error !== "string") return false;
  return true;
}

function isValidSidebarEventEnvelope(
  data: unknown,
): data is SidebarEventEnvelope {
  if (typeof data !== "object" || data === null) return false;
  const obj = data as Record<string, unknown>;
  if (typeof obj.seq !== "number") return false;
  if (typeof obj.category !== "string") return false;

  switch (obj.category) {
    case "snapshot":
      return isValidSnapshot(obj);
    case "loading_phase":
      return isValidLoadingPhase(obj);
    case "workspace_snapshot":
      return Array.isArray(obj.workspace_roots);
    case "trajectories_snapshot":
      return Array.isArray(obj.trajectories);
    case "tasks_snapshot":
      return Array.isArray(obj.tasks);
    case "buddy_snapshot":
      return "buddy" in obj;
    case "trajectory":
      return isValidTrajectoryEvent(obj);
    case "task":
      return isValidTaskEvent(obj);
    case "notification":
      return isValidNotificationEvent(obj);
    case "buddy":
      return typeof obj.buddy_event === "object" && obj.buddy_event !== null;
    default:
      return false;
  }
}

const IDLE_TIMEOUT_MS = 30_000;

export function subscribeToSidebarEvents(
  port: number,
  apiKey: string | null,
  callbacks: SidebarSubscriptionCallbacks,
): () => void {
  const url = `http://127.0.0.1:${port}/v1/sidebar/subscribe`;
  const abortController = new AbortController();
  const state = { connected: false, lastSeq: -1, aborted: false };
  let idleTimer: ReturnType<typeof setTimeout> | null = null;

  const headers: Record<string, string> = {};
  if (apiKey) {
    headers.Authorization = `Bearer ${apiKey}`;
  }

  const resetIdleTimer = () => {
    if (idleTimer) clearTimeout(idleTimer);
    idleTimer = setTimeout(() => {
      abortController.abort();
    }, IDLE_TIMEOUT_MS);
  };

  const cleanup = () => {
    if (idleTimer) {
      clearTimeout(idleTimer);
      idleTimer = null;
    }
    if (!state.aborted) {
      state.aborted = true;
      abortController.abort();
    }
    if (state.connected) {
      state.connected = false;
      callbacks.onDisconnected?.();
    }
  };

  void fetch(url, {
    method: "GET",
    headers,
    signal: abortController.signal,
  })
    .then(async (response) => {
      if (!response.ok) {
        throw new Error(`SSE connection failed: ${response.status}`);
      }
      if (!response.body) {
        throw new Error("Response body is null");
      }

      state.connected = true;
      callbacks.onConnected?.();
      resetIdleTimer();

      const reader = response.body.getReader();
      const decoder = new TextDecoder();
      let buffer = "";

      try {
        for (;;) {
          const { done, value } = await reader.read();
          if (done) break;

          resetIdleTimer();
          buffer += decoder.decode(value, { stream: true });
          buffer = buffer.replace(/\r\n/g, "\n").replace(/\r/g, "\n");

          const blocks = buffer.split("\n\n");
          buffer = blocks.pop() ?? "";

          for (const block of blocks) {
            if (!block.trim()) continue;
            if (block.startsWith(":")) continue;

            const dataLines: string[] = [];
            for (const rawLine of block.split("\n")) {
              if (rawLine.startsWith(":")) continue;
              if (!rawLine.startsWith("data:")) continue;
              dataLines.push(rawLine.slice(5).replace(/^\s*/, ""));
            }

            if (dataLines.length === 0) continue;

            const dataStr = dataLines.join("\n");
            if (dataStr === "[DONE]") continue;

            let parsed: unknown;
            try {
              parsed = JSON.parse(dataStr);
            } catch (e) {
              const msg = e instanceof Error ? e.message : "JSON parse error";
              throw new Error(`Parse error: ${msg}`);
            }

            if (!isValidSidebarEventEnvelope(parsed)) {
              throw new Error("Invalid event structure");
            }

            if (parsed.category === "snapshot") {
              state.lastSeq = parsed.seq;
            } else if (state.lastSeq >= 0 && parsed.seq !== state.lastSeq + 1) {
              throw new Error(
                `Seq gap: expected ${state.lastSeq + 1}, got ${parsed.seq}`,
              );
            } else {
              state.lastSeq = parsed.seq;
            }

            callbacks.onEvent(parsed);
          }
        }
      } finally {
        await reader.cancel().catch(() => {
          // Ignore cancel errors - connection already closed
        });
      }

      cleanup();
    })
    .catch((err: unknown) => {
      const error = err as Error;
      if (error.name !== "AbortError") {
        callbacks.onError(error);
      }
      cleanup();
    });

  return cleanup;
}
