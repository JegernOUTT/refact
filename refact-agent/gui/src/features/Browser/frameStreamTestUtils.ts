import { HttpResponse } from "msw";
import { vi } from "vitest";
import {
  FRAME_STREAM_MIME,
  type BrowserFrameHeader,
} from "../../services/refact/projectBrowserStream";

export function frameStreamRecord(header: unknown, bytes = new Uint8Array()) {
  const json = new TextEncoder().encode(JSON.stringify(header));
  const record = new Uint8Array(8 + json.length + bytes.length);
  const lengths = new DataView(record.buffer);
  lengths.setUint32(0, json.length);
  lengths.setUint32(4, bytes.length);
  record.set(json, 8);
  record.set(bytes, 8 + json.length);
  return record;
}

export function frameRecord(overrides: Partial<BrowserFrameHeader> = {}) {
  return frameStreamRecord(
    {
      type: "frame",
      runtime_id: "runtime-a",
      tab_id: "tab-b",
      mime: "image/jpeg",
      url: "https://example.test/b",
      title: "Example B",
      width: 600,
      height: 300,
      viewport: { width: 1200, height: 600, device_scale_factor: 1 },
      ...overrides,
    },
    Uint8Array.of(0xff, 0xd8, 0xff, 0xd9),
  );
}

export function frameStreamResponse(
  overrides: Partial<BrowserFrameHeader> = {},
) {
  return new HttpResponse(
    new ReadableStream({
      start(controller) {
        controller.enqueue(frameRecord(overrides));
      },
    }),
    { headers: { "Content-Type": FRAME_STREAM_MIME } },
  );
}

export function mockFrameDecoder() {
  let id = 0;
  const create = vi
    .spyOn(URL, "createObjectURL")
    .mockImplementation(() => `blob:http://localhost/frame-${++id}`);
  const revoke = vi
    .spyOn(URL, "revokeObjectURL")
    .mockImplementation(() => undefined);
  const decode = vi
    .spyOn(HTMLImageElement.prototype, "decode")
    .mockResolvedValue(undefined);
  vi.spyOn(HTMLImageElement.prototype, "naturalWidth", "get").mockReturnValue(
    600,
  );
  vi.spyOn(HTMLImageElement.prototype, "naturalHeight", "get").mockReturnValue(
    300,
  );
  return { create, revoke, decode };
}
