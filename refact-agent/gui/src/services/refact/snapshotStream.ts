import { WorkFrameDecoder } from "./workSse";
import { SnapshotVisibilityError, type SnapshotFailure } from "./snapshotError";

export type SnapshotStreamOptions<T> = {
  timeoutMs: number;
  /** Parses one SSE block. Throws when the stream must be abandoned. */
  parse: (block: string) => T | null;
  /** "reset" abandons the stream so the caller reconnects for a fresh snapshot. */
  sequence: (previous: T | null, next: T) => "accept" | "duplicate" | "reset";
  onSnapshot: (snapshot: T) => void;
  onError: (reason: string, failure?: SnapshotFailure) => void;
  unavailable: string;
  timedOut: string;
  disconnected: string;
  outOfSequence: string;
};

export function subscribeToSnapshots<T>(
  url: string,
  token: string | null,
  options: SnapshotStreamOptions<T>,
): () => void {
  const abort = new AbortController();
  const state = { stopped: false };
  const isStopped = () => state.stopped;
  let previous: T | null = null;
  let timeout: ReturnType<typeof setTimeout> | undefined;
  const fail = (reason: string, failure: SnapshotFailure) => {
    if (state.stopped) return;
    state.stopped = true;
    abort.abort();
    options.onError(reason, failure);
  };
  const resetTimeout = () => {
    clearTimeout(timeout);
    timeout = setTimeout(
      () => fail(options.timedOut, { kind: "transport" }),
      options.timeoutMs,
    );
  };
  const consumeBlock = (block: string) => {
    let snapshot: T | null;
    try {
      snapshot = options.parse(block);
    } catch (error) {
      fail(
        options.unavailable,
        error instanceof SnapshotVisibilityError
          ? error.failure
          : { kind: "malformed", status: "CUSTOM_ERROR" },
      );
      return;
    }
    if (!snapshot) return;
    const action = options.sequence(previous, snapshot);
    if (action === "reset") {
      fail(options.outOfSequence, { kind: "reset" });
      return;
    }
    if (action === "accept") {
      previous = snapshot;
      options.onSnapshot(snapshot);
    }
  };
  resetTimeout();
  const headers: Record<string, string> = { Accept: "text/event-stream" };
  if (token) headers.Authorization = `Bearer ${token}`;
  void (async () => {
    try {
      const response = await fetch(url, { headers, signal: abort.signal });
      if (!response.ok) {
        fail(options.unavailable, { kind: "http", status: response.status });
        return;
      }
      if (!response.body) {
        fail(options.unavailable, {
          kind: "malformed",
          status: "CUSTOM_ERROR",
        });
        return;
      }
      const reader = response.body.getReader();
      const decoder = new WorkFrameDecoder();
      try {
        for (;;) {
          const { done, value } = await reader.read();
          if (state.stopped) return;
          if (done) throw new Error(options.disconnected);
          resetTimeout();
          for (const block of decoder.push(value)) {
            if (isStopped()) return;
            consumeBlock(block);
          }
        }
      } finally {
        await reader.cancel().catch(() => undefined);
      }
    } catch {
      fail(options.disconnected, { kind: "transport" });
    } finally {
      clearTimeout(timeout);
    }
  })();
  return () => {
    state.stopped = true;
    clearTimeout(timeout);
    abort.abort();
  };
}
