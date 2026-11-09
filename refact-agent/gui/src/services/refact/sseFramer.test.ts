import { describe, expect, it, vi } from "vitest";
import { SseFramer } from "./sseFramer";
import source from "./sseFramer.ts?raw";

describe("SseFramer", () => {
  it.each(["\n", "\r\n", "\r"])(
    "frames %j lines at every possible chunk boundary",
    (newline) => {
      const input = [
        "data: first",
        "data: second",
        "",
        ": heartbeat",
        "",
        "event: ignored",
        "",
        "data: third",
        "",
        "",
      ].join(newline);
      for (let boundary = 0; boundary <= input.length; boundary++) {
        const framer = new SseFramer();
        expect([
          ...framer.push(input.slice(0, boundary)),
          ...framer.push(""),
          ...framer.push(input.slice(boundary)),
        ]).toEqual([
          "data: first\ndata: second",
          ": heartbeat",
          "event: ignored",
          "data: third",
        ]);
      }
      const framer = new SseFramer();
      expect([...input].flatMap((char) => [...framer.push(char)])).toEqual([
        "data: first\ndata: second",
        ": heartbeat",
        "event: ignored",
        "data: third",
      ]);
    },
  );

  it("handles mixed newlines and retains unterminated frames", () => {
    const framer = new SseFramer();
    expect([
      ...framer.push("\r\ndata: one\r\ndata: two\r\rdata: tail"),
    ]).toEqual(["data: one\ndata: two"]);
    expect([...framer.push("\n")]).toEqual([]);
    expect([...framer.push("\n")]).toEqual(["data: tail"]);
  });

  it("stores large incomplete frames as segments without joining or splitting", () => {
    const framer = new SseFramer();
    const chunk = "x".repeat(4096);
    const join = vi.spyOn(Array.prototype, "join");
    const split = vi.spyOn(String.prototype, "split");
    let frames = 0;
    for (let i = 0; i < 512; i++) {
      for (const _frame of framer.push(chunk)) frames++;
    }
    const incompleteJoins = join.mock.calls.length;
    const incompleteSplits = split.mock.calls.length;
    const completed = [...framer.push("\n\n")];
    const completeJoins = join.mock.calls.length;
    join.mockRestore();
    split.mockRestore();
    expect(frames).toBe(0);
    expect(incompleteJoins).toBe(0);
    expect(incompleteSplits).toBe(0);
    expect(completeJoins).toBe(1);
    expect(completed).toEqual([chunk.repeat(512)]);
    expect(source).toContain("i < chunk.length");
    expect(source).toContain("chunk.charCodeAt(i)");
    expect(source).not.toMatch(/segments\s*\+=|\.split\(|\.indexOf\(/);
  });
});
