import { describe, expect, it } from "vitest";
import {
  MAX_BUFFERED_STREAM_TEXT_UNITS,
  streamDeltaFlushDelayMs,
  streamDeltaTextUnits,
  subchatFlushDelayMs,
} from "./chatStreamBatching";

describe("chat stream batching policy", () => {
  it("keeps active normal deltas within the 50 ms visibility budget", () => {
    expect(streamDeltaFlushDelayMs(true, 0)).toBeLessThanOrEqual(50);
    expect(streamDeltaFlushDelayMs(true, 8_191)).toBeLessThanOrEqual(50);
  });

  it("keeps active large deltas and subchat updates within 150 ms", () => {
    expect(streamDeltaFlushDelayMs(true, 8_192)).toBe(125);
    expect(streamDeltaFlushDelayMs(true, 50 * 1024 * 1024)).toBeLessThanOrEqual(
      150,
    );
    expect(subchatFlushDelayMs(true)).toBe(125);
  });

  it("keeps background flushes bounded without promoting them to active cadence", () => {
    expect(streamDeltaFlushDelayMs(false, 0)).toBe(750);
    expect(streamDeltaFlushDelayMs(false, 50 * 1024 * 1024)).toBe(750);
    expect(subchatFlushDelayMs(false)).toBe(750);
  });

  it("counts streamed Unicode text in JavaScript string units", () => {
    expect(
      streamDeltaTextUnits([
        { op: "append_content", text: "🐛" },
        { op: "append_reasoning", text: "🧠" },
        { op: "set_reasoning", text: "é" },
        { op: "set_usage", usage: { completion_tokens: 3 } },
      ]),
    ).toBe("🐛🧠é".length);
  });

  it("retains the bounded pending stream cap", () => {
    expect(MAX_BUFFERED_STREAM_TEXT_UNITS).toBe(2_000_000);
  });
});
