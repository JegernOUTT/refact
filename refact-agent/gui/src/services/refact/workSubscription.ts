import { workSequence, type WorkPage } from "./workTypes";
import { parseWorkFrame, WorkFrameDecoder } from "./workSse";

export type WorkStreamOptions = {
  maxBytes: number;
  timeoutMs: number;
  onSnapshot: (page: WorkPage) => void;
  onError: (reason: string) => void;
};

export function subscribeToWork(
  url: string,
  token: string | null,
  options: WorkStreamOptions,
): () => void {
  const abort = new AbortController();
  const state = { stopped: false };
  const isStopped = () => state.stopped;
  let previous: WorkPage | null = null;
  let timeout: ReturnType<typeof setTimeout> | undefined;
  const fail = (reason: string) => {
    if (state.stopped) return;
    state.stopped = true;
    abort.abort();
    options.onError(reason);
  };
  const resetTimeout = () => {
    clearTimeout(timeout);
    timeout = setTimeout(
      () => fail("Work stream timed out"),
      options.timeoutMs,
    );
  };
  const consumeBlock = (block: string) => {
    const page = parseWorkFrame(block);
    if (!page) return;
    const action = workSequence(previous, page);
    if (action === "reset")
      throw new Error("Work sequence changed; fresh snapshot required");
    if (action === "accept") {
      previous = page;
      options.onSnapshot(page);
    }
  };
  resetTimeout();
  const headers: Record<string, string> = { Accept: "text/event-stream" };
  if (token) headers.Authorization = `Bearer ${token}`;
  void (async () => {
    try {
      const response = await fetch(url, { headers, signal: abort.signal });
      if (!response.ok || !response.body)
        throw new Error("Work stream unavailable");
      const reader = response.body.getReader();
      const decoder = new WorkFrameDecoder(options.maxBytes);
      try {
        for (;;) {
          const { done, value } = await reader.read();
          if (state.stopped) return;
          if (done) throw new Error("Work stream disconnected");
          resetTimeout();
          for (const block of decoder.push(value)) {
            if (isStopped()) return;
            consumeBlock(block);
          }
        }
      } finally {
        await reader.cancel().catch(() => undefined);
      }
    } catch (error) {
      fail(error instanceof Error ? error.message : "Work stream unavailable");
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
