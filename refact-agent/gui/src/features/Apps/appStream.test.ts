import { describe, expect, it } from "vitest";
import { readAppStream, type AppStreamEvent } from "./appStream";

function streamResponse(chunks: Uint8Array[]): Response {
  let index = 0;
  const body = new ReadableStream<Uint8Array>({
    pull(controller) {
      if (index < chunks.length) controller.enqueue(chunks[index++]);
      else controller.close();
    },
  });
  return new Response(body, { status: 200 });
}

function chunked(text: string, size: number): Uint8Array[] {
  const bytes = new TextEncoder().encode(text);
  const chunks: Uint8Array[] = [];
  for (let i = 0; i < bytes.length; i += size)
    chunks.push(bytes.slice(i, i + size));
  return chunks;
}

const frame = (event: Record<string, unknown>, eol = "\n") =>
  `data: ${JSON.stringify(event)}${eol}${eol}`;

describe("readAppStream", () => {
  it("decodes frames larger than the former 2 MiB cap with exact content", async () => {
    const big = "é".repeat(1_500_000) + "x".repeat(1_000_000);
    const events = [
      { generation: "g1", seq: 0, kind: "snapshot", data: { apps: [] } },
      { generation: "g1", seq: 1, kind: "app_state", data: { value: big } },
      { generation: "g1", seq: 2, kind: "app_event", data: { tail: "ok" } },
    ];
    const text =
      frame(events[0]) + frame(events[1], "\r\n") + frame(events[2], "\r\n");
    expect(text.length).toBeGreaterThan(2 * 1024 * 1024);
    expect(new TextEncoder().encode(text).length).toBeGreaterThan(
      3 * 1024 * 1024,
    );
    const received: AppStreamEvent[] = [];
    await readAppStream(
      streamResponse(chunked(text, 65_537)),
      new AbortController().signal,
      (event) => received.push(event),
    );
    expect(received).toEqual(events);
    expect((received[1].data as { value: string }).value).toBe(big);
  });

  it("joins CRLF delimiters split across chunks", async () => {
    const events = [
      { generation: "g", seq: 5, kind: "reset", data: null },
      { generation: "g", seq: 6, kind: "app_event", data: "a" },
    ];
    const text = frame(events[0], "\r\n") + frame(events[1], "\r\n");
    const received: AppStreamEvent[] = [];
    await readAppStream(
      streamResponse(chunked(text, 1)),
      new AbortController().signal,
      (event) => received.push(event),
    );
    expect(received).toEqual(events);
  });

  it("still rejects invalid envelopes, malformed JSON and sequence gaps", async () => {
    const signal = new AbortController().signal;
    await expect(
      readAppStream(
        streamResponse(chunked(frame({ seq: 0, kind: "snapshot" }), 7)),
        signal,
        () => undefined,
      ),
    ).rejects.toThrow("Invalid app stream envelope");
    await expect(
      readAppStream(
        streamResponse(chunked("data: {oops\n\n", 3)),
        signal,
        () => undefined,
      ),
    ).rejects.toThrow(SyntaxError);
    const gap =
      frame({ generation: "g", seq: 0, kind: "snapshot", data: null }) +
      frame({ generation: "g", seq: 2, kind: "app_event", data: null });
    const received: AppStreamEvent[] = [];
    await expect(
      readAppStream(streamResponse(chunked(gap, 5)), signal, (event) =>
        received.push(event),
      ),
    ).rejects.toThrow("App stream sequence gap");
    expect(received).toHaveLength(1);
  });

  it("stops reading once aborted", async () => {
    const abort = new AbortController();
    const received: AppStreamEvent[] = [];
    const text =
      frame({ generation: "g", seq: 0, kind: "snapshot", data: null }) +
      frame({ generation: "g", seq: 1, kind: "app_event", data: null });
    await readAppStream(
      streamResponse(chunked(text, text.indexOf("\n\n") + 2)),
      abort.signal,
      (event) => {
        received.push(event);
        abort.abort();
      },
    );
    expect(received).toHaveLength(1);
  });
});
