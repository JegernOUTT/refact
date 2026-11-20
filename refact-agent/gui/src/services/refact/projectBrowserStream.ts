import {
  prepareBrowserHeaders,
  type ProjectBrowserInputFence,
} from "./browser";

export const FRAME_STREAM_MIME = "application/x-refact-browser-frames";
const MAX_HEADER = 32 * 1024;
const MAX_PAYLOAD = 4 * 1024 * 1024;
const IDLE_TIMEOUT_MS = 30_000;

export type BrowserFrameHeader = ProjectBrowserInputFence & {
  type: "frame";
  mime: "image/jpeg";
  url: string;
  title: string;
  width: number;
  height: number;
  viewport: { width: number; height: number; device_scale_factor: number };
};

export type ReceivedBrowserFrame = BrowserFrameHeader & {
  bytes: Uint8Array<ArrayBuffer>;
};

class FrameStreamError extends Error {
  constructor(
    readonly status: number,
    detail?: string,
  ) {
    super(detail ?? `Browser frame stream failed (${status})`);
  }
}

const invalid = () => new Error("Invalid browser frame stream record");
const isRecord = (v: unknown): v is Record<string, unknown> =>
  typeof v === "object" && v !== null && !Array.isArray(v);
const positive = (n: unknown): n is number =>
  typeof n === "number" && Number.isFinite(n) && n > 0;

function frameHeader(
  value: Record<string, unknown>,
  target: ProjectBrowserInputFence,
): BrowserFrameHeader {
  const { width, height, url, title, viewport } = value;
  if (
    value.type !== "frame" ||
    value.mime !== "image/jpeg" ||
    value.runtime_id !== target.runtime_id ||
    value.tab_id !== target.tab_id ||
    !positive(width) ||
    !Number.isSafeInteger(width) ||
    !positive(height) ||
    !Number.isSafeInteger(height) ||
    typeof url !== "string" ||
    typeof title !== "string" ||
    !isRecord(viewport) ||
    !positive(viewport.width) ||
    !positive(viewport.height) ||
    !positive(viewport.device_scale_factor)
  )
    throw invalid();
  return {
    ...target,
    type: "frame",
    mime: "image/jpeg",
    width,
    height,
    url,
    title,
    viewport: {
      width: viewport.width,
      height: viewport.height,
      device_scale_factor: viewport.device_scale_factor,
    },
  };
}

// Keep one transport chunk and fill only the current, length-checked field. Never
// concatenate the unread stream or retain a large network buffer with a frame.
function fieldReader(
  reader: ReadableStreamDefaultReader<Uint8Array>,
  signal: AbortSignal,
) {
  let chunk: Uint8Array | undefined;
  let offset = 0;
  return async (length: number) => {
    const bytes = new Uint8Array(length);
    let filled = 0;
    while (filled < length) {
      signal.throwIfAborted();
      if (!chunk || offset === chunk.length) {
        const next = await reader.read();
        signal.throwIfAborted();
        if (next.done)
          throw new Error("Browser frame stream ended unexpectedly");
        chunk = next.value;
        offset = 0;
      }
      const count = Math.min(length - filled, chunk.length - offset);
      bytes.set(chunk.subarray(offset, offset + count), filled);
      offset += count;
      filled += count;
    }
    return bytes;
  };
}

export async function subscribeProjectBrowserFrames(
  url: string,
  apiKey: string | null | undefined,
  target: ProjectBrowserInputFence,
  signal: AbortSignal,
  onFrame: (frame: ReceivedBrowserFrame) => void,
) {
  signal.throwIfAborted();
  const controller = new AbortController();
  let reader: ReadableStreamDefaultReader<Uint8Array> | undefined;
  const cancelReader = () => {
    void reader?.cancel().catch(() => undefined);
  };
  const abort = () => controller.abort();
  signal.addEventListener("abort", abort);
  controller.signal.addEventListener("abort", cancelReader);
  let timer: number | undefined;
  const resetTimeout = () => {
    window.clearTimeout(timer);
    timer = window.setTimeout(
      () => controller.abort(new Error("Browser frame stream timed out")),
      IDLE_TIMEOUT_MS,
    );
  };
  try {
    resetTimeout();
    const headers = prepareBrowserHeaders(
      new Headers({
        "Content-Type": "application/json",
        Accept: FRAME_STREAM_MIME,
      }),
      apiKey,
    );
    const response = await fetch(url, {
      method: "POST",
      headers,
      body: JSON.stringify(target),
      signal: controller.signal,
    });
    reader = response.body?.getReader();
    controller.signal.throwIfAborted();
    if (!response.ok) throw new FrameStreamError(response.status);
    if (
      !reader ||
      response.headers
        .get("content-type")
        ?.split(";")[0]
        .trim()
        .toLowerCase() !== FRAME_STREAM_MIME
    )
      throw new Error("Expected a browser frame stream");
    const read = fieldReader(reader, controller.signal);
    const decoder = new TextDecoder("utf-8", { fatal: true });
    for (;;) {
      const lengths = new DataView((await read(8)).buffer);
      const headerLength = lengths.getUint32(0);
      const payloadLength = lengths.getUint32(4);
      if (
        !headerLength ||
        headerLength > MAX_HEADER ||
        payloadLength > MAX_PAYLOAD
      )
        throw invalid();
      const header: unknown = JSON.parse(
        decoder.decode(await read(headerLength)),
      );
      if (!isRecord(header)) throw invalid();
      if (header.type === "frame") {
        const frame = frameHeader(header, target);
        if (payloadLength < 4) throw invalid();
        const bytes = await read(payloadLength);
        if (
          bytes[0] !== 0xff ||
          bytes[1] !== 0xd8 ||
          bytes[payloadLength - 2] !== 0xff ||
          bytes[payloadLength - 1] !== 0xd9
        )
          throw invalid();
        controller.signal.throwIfAborted();
        onFrame({ ...frame, bytes });
      } else {
        if (payloadLength !== 0) throw invalid();
        if (
          header.type === "error" &&
          typeof header.status === "number" &&
          Number.isInteger(header.status) &&
          header.status >= 400 &&
          header.status <= 599 &&
          typeof header.detail === "string"
        )
          throw new FrameStreamError(header.status, header.detail);
        if (header.type !== "heartbeat") throw invalid();
      }
      resetTimeout();
    }
  } catch (error) {
    controller.signal.throwIfAborted();
    throw error;
  } finally {
    window.clearTimeout(timer);
    signal.removeEventListener("abort", abort);
    controller.signal.removeEventListener("abort", cancelReader);
    cancelReader();
    reader?.releaseLock();
    controller.abort();
  }
}
