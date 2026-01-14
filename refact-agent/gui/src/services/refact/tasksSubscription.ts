import type { TaskMeta, TaskBoard } from "./tasks";

export type TaskEvent =
  | { type: "snapshot"; tasks: TaskMeta[] }
  | { type: "task_created"; task_id: string; meta: TaskMeta }
  | { type: "task_updated"; task_id: string; meta: TaskMeta }
  | { type: "task_deleted"; task_id: string }
  | { type: "board_changed"; task_id: string; rev: number; board: TaskBoard };

export type TaskEventEnvelope = {
  seq: number;
} & TaskEvent;

export type TaskSubscriptionCallbacks = {
  onEvent: (event: TaskEventEnvelope) => void;
  onError: (error: Error) => void;
  onConnected?: () => void;
  onDisconnected?: () => void;
};

export function subscribeToTaskEvents(
  port: number,
  apiKey: string | null,
  callbacks: TaskSubscriptionCallbacks,
): () => void {
  const url = `http://127.0.0.1:${port}/v1/tasks/subscribe`;
  const abortController = new AbortController();
  const state = { connected: false, lastSeq: -1 };

  const headers: Record<string, string> = {};
  if (apiKey) {
    headers.Authorization = `Bearer ${apiKey}`;
  }

  const disconnect = () => {
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

      const reader = response.body.getReader();
      const decoder = new TextDecoder();
      let buffer = "";

      for (;;) {
        const { done, value } = await reader.read();
        if (done) break;

        buffer += decoder.decode(value, { stream: true });
        buffer = buffer.replace(/\r\n/g, "\n").replace(/\r/g, "\n");

        const blocks = buffer.split("\n\n");
        buffer = blocks.pop() ?? "";

        for (const block of blocks) {
          if (!block.trim()) continue;

          const dataLines: string[] = [];
          for (const rawLine of block.split("\n")) {
            if (!rawLine.startsWith("data:")) continue;
            dataLines.push(rawLine.slice(5).replace(/^\s*/, ""));
          }

          if (dataLines.length === 0) continue;

          const dataStr = dataLines.join("\n");
          if (dataStr === "[DONE]") continue;

          try {
            const parsed = JSON.parse(dataStr) as unknown;
            if (isValidTaskEventEnvelope(parsed)) {
              if (parsed.type === "snapshot") {
                state.lastSeq = parsed.seq;
              } else if (state.lastSeq >= 0 && parsed.seq !== state.lastSeq + 1) {
                throw new Error(`Seq gap: expected ${state.lastSeq + 1}, got ${parsed.seq}`);
              } else {
                state.lastSeq = parsed.seq;
              }
              callbacks.onEvent(parsed);
            }
          } catch {
            disconnect();
            callbacks.onError(new Error("Sequence gap or parse error"));
            return;
          }
        }
      }

      disconnect();
    })
    .catch((err: unknown) => {
      const error = err as Error;
      if (error.name !== "AbortError") {
        callbacks.onError(error);
        disconnect();
      }
    });

  return () => {
    abortController.abort();
    disconnect();
  };
}

function isValidTaskEventEnvelope(data: unknown): data is TaskEventEnvelope {
  if (typeof data !== "object" || data === null) return false;
  const obj = data as Record<string, unknown>;
  if (typeof obj.seq !== "number") return false;
  if (typeof obj.type !== "string") return false;
  return ["snapshot", "task_created", "task_updated", "task_deleted", "board_changed"].includes(obj.type);
}
