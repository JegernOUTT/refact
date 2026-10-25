import { describe, expect, it } from "vitest";
import { WorkFrameDecoder, parseWorkFrame } from "./workSse";
import { workFixture } from "./workFixtures";

const bytes = (text: string) => new TextEncoder().encode(text);

describe("Work SSE frames", () => {
  it("decodes split UTF-8 and CRLF without changing snapshot text", () => {
    const page = { ...workFixture, project: "project-猫" };
    const input = bytes(
      `event: snapshot\r\ndata: ${JSON.stringify(page)}\r\n\r\n`,
    );
    const decoder = new WorkFrameDecoder();
    const blocks = Array.from(input).flatMap((byte) => [
      ...decoder.push(Uint8Array.of(byte)),
    ]);
    expect(blocks).toHaveLength(1);
    expect(parseWorkFrame(blocks[0])).toEqual(page);
  });

  it("ignores heartbeat frames and fails closed without reflecting payloads", () => {
    expect(parseWorkFrame(": keep alive")).toBeNull();
    for (const block of [
      "event: snapshot\ndata: secret-invalid-json",
      'event: snapshot\ndata: {"secret":true}',
      `event: unknown\ndata: ${JSON.stringify(workFixture)}`,
    ]) {
      expect(() => parseWorkFrame(block)).toThrow("Invalid Work snapshot");
    }
    expect(() => parseWorkFrame("event: work_error\ndata: secret")).toThrow(
      "Work visibility changed or service became unavailable",
    );
  });
});
