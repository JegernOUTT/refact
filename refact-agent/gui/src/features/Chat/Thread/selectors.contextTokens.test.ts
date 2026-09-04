import { describe, expect, it } from "vitest";
import type { RootState } from "../../../app/store";
import { selectLastAssistantMessageWithTokensById } from "./selectors";

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
