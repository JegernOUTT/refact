import { describe, expect, it } from "vitest";
import type { RootState } from "../../../app/store";
import {
  selectLastAssistantMessageWithTokensById,
  selectThreadTotalUsageById,
} from "./selectors";

function stateWithMessages(messages: unknown[]): RootState {
  return {
    chat: {
      current_thread_id: "thread-A",
      open_thread_ids: ["thread-A"],
      threads: {
        "thread-A": { thread: { id: "thread-A", messages } },
      },
    },
  } as unknown as RootState;
}

describe("selectLastAssistantMessageWithTokensById", () => {
  it("keeps the reported context size while a newer assistant message has no usage yet", () => {
    const state = stateWithMessages([
      {
        role: "assistant",
        content: "answered",
        usage: { prompt_tokens: 1200, completion_tokens: 20 },
      },
      { role: "assistant", content: "" },
    ]);

    const message = selectLastAssistantMessageWithTokensById(state, "thread-A");

    expect(message?.usage?.prompt_tokens).toBe(1200);
  });

  it("counts cache tokens so a fully cached turn still reports a context size", () => {
    const state = stateWithMessages([
      {
        role: "assistant",
        content: "cached",
        usage: {
          prompt_tokens: 0,
          completion_tokens: 5,
          cache_read_input_tokens: 800,
        },
      },
      { role: "assistant", content: "" },
    ]);

    const message = selectLastAssistantMessageWithTokensById(state, "thread-A");

    expect(message?.usage?.cache_read_input_tokens).toBe(800);
  });

  it("skips assistant messages that reported no input tokens at all", () => {
    const state = stateWithMessages([
      {
        role: "assistant",
        content: "older",
        usage: { prompt_tokens: 640, completion_tokens: 4 },
      },
      {
        role: "assistant",
        content: "zeroed",
        usage: { prompt_tokens: 0, completion_tokens: 0 },
      },
    ]);

    const message = selectLastAssistantMessageWithTokensById(state, "thread-A");

    expect(message?.usage?.prompt_tokens).toBe(640);
  });

  it("returns nothing when no assistant message ever reported tokens", () => {
    const state = stateWithMessages([
      { role: "user", content: "hi" },
      { role: "assistant", content: "" },
    ]);

    expect(
      selectLastAssistantMessageWithTokensById(state, "thread-A"),
    ).toBeUndefined();
  });
});

describe("selectThreadTotalUsageById", () => {
  it("sums usage across every assistant turn, not just the newest one", () => {
    const state = stateWithMessages([
      { role: "user", content: "hi" },
      {
        role: "assistant",
        content: "first",
        usage: {
          prompt_tokens: 100,
          completion_tokens: 10,
          cache_read_input_tokens: 900,
          cache_creation_input_tokens: 50,
        },
      },
      { role: "user", content: "again" },
      {
        role: "assistant",
        content: "second",
        usage: {
          prompt_tokens: 20,
          completion_tokens: 7,
          cache_read_input_tokens: 1100,
          cache_creation_input_tokens: 5,
        },
      },
    ]);

    const usage = selectThreadTotalUsageById(state, "thread-A");

    expect(usage?.prompt_tokens).toBe(120);
    expect(usage?.completion_tokens).toBe(17);
    expect(usage?.cache_read_input_tokens).toBe(2000);
    expect(usage?.cache_creation_input_tokens).toBe(55);
  });

  it("accumulates provider-aliased cache fields too", () => {
    const state = stateWithMessages([
      {
        role: "assistant",
        content: "first",
        usage: {
          prompt_tokens: 5,
          completion_tokens: 1,
          cache_read_tokens: 400,
          cache_creation_tokens: 20,
        },
      },
      {
        role: "assistant",
        content: "second",
        usage: {
          prompt_tokens: 5,
          completion_tokens: 1,
          cache_read_tokens: 600,
          cache_creation_tokens: 30,
        },
      },
    ]);

    const usage = selectThreadTotalUsageById(state, "thread-A");

    expect(usage?.cache_read_input_tokens).toBe(1000);
    expect(usage?.cache_creation_input_tokens).toBe(50);
  });

  it("returns nothing when no assistant message reported usage", () => {
    const state = stateWithMessages([
      { role: "user", content: "hi" },
      { role: "assistant", content: "" },
    ]);

    expect(selectThreadTotalUsageById(state, "thread-A")).toBeUndefined();
  });
});
