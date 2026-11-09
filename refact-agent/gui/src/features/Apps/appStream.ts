import { isAppRecord } from "../../services/refact/apps";
export type AppStreamEvent = {
  hostChatId?: string | null;
  generation: string;
  seq: number;
  kind: string;
  data: unknown;
};
const listeners = new Set<(event: AppStreamEvent) => void>();
export function listenAppEvents(listener: (event: AppStreamEvent) => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}
export async function readAppStream(
  response: Response,
  signal: AbortSignal,
  onEvent: (event: AppStreamEvent) => void,
): Promise<void> {
  if (!response.ok || !response.body) throw new Error("App stream unavailable");
  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  let generation: string | null = null;
  let seq = 0;
  try {
    while (!signal.aborted) {
      const chunk = await reader.read();
      if (chunk.done) break;
      buffer += decoder
        .decode(chunk.value, { stream: true })
        .replace(/\r\n/g, "\n");
      if (buffer.length > 2 * 1024 * 1024)
        throw new Error("App stream frame too large");
      let end: number;
      while ((end = buffer.indexOf("\n\n")) >= 0) {
        const frame = buffer.slice(0, end);
        buffer = buffer.slice(end + 2);
        const data = frame
          .split("\n")
          .filter((line) => line.startsWith("data:"))
          .map((line) => line.slice(5).trimStart())
          .join("\n");
        if (!data) continue;
        const event: unknown = JSON.parse(data);
        if (
          !isAppRecord(event) ||
          typeof event.generation !== "string" ||
          !Number.isSafeInteger(event.seq) ||
          typeof event.kind !== "string"
        )
          throw new Error("Invalid app stream envelope");
        const next = event as AppStreamEvent;
        if (next.kind === "snapshot" || next.kind === "reset") {
          generation = next.generation;
          seq = next.seq;
        } else {
          if (generation !== next.generation || next.seq !== seq + 1)
            throw new Error("App stream sequence gap");
          seq = next.seq;
        }
        onEvent(next);
      }
    }
  } finally {
    await reader.cancel().catch(() => undefined);
    reader.releaseLock();
  }
}
export function publishAppEvent(event: AppStreamEvent) {
  listeners.forEach((listener) => listener(event));
}
