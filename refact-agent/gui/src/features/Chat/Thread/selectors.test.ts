import { describe, expect, it } from "vitest";
import type { RootState } from "../../../app/store";
import type { ChatMessages } from "../../../services/refact/types";
import {
  selectAutoCompactEnabled,
  selectAutoCompactEnabledById,
  selectAutoCompressionCap,
  selectAutoCompressionCapById,
  selectSynthesizedPlanText,
  selectMessagesById,
} from "./selectors";

function makeState(
  autoCompactEnabled?: boolean,
  autoCompressionCap?: number,
): RootState {
  const thread =
    autoCompactEnabled === undefined
      ? {}
      : { auto_compact_enabled: autoCompactEnabled };
  if (autoCompressionCap !== undefined) {
    Object.assign(thread, { auto_compression_cap: autoCompressionCap });
  }

  return {
    chat: {
      current_thread_id: "chat-1",
      threads: {
        "chat-1": {
          thread,
        },
      },
    },
  } as unknown as RootState;
}

describe("auto compact selectors", () => {
  it("default to enabled when missing", () => {
    const state = makeState();

    expect(selectAutoCompactEnabled(state)).toBe(true);
    expect(selectAutoCompactEnabledById(state, "chat-1")).toBe(true);
  });

  it("return false when explicitly disabled", () => {
    const state = makeState(false);

    expect(selectAutoCompactEnabled(state)).toBe(false);
    expect(selectAutoCompactEnabledById(state, "chat-1")).toBe(false);
  });
});

describe("auto compression cap selectors", () => {
  it("return the current and scoped thread values", () => {
    const state = makeState(undefined, 8192);

    expect(selectAutoCompressionCap(state)).toBe(8192);
    expect(selectAutoCompressionCapById(state, "chat-1")).toBe(8192);
  });
});

describe("reconstructed plan selectors", () => {
  it("uses only the latest report plan and suffix deltas while preserving the archive", () => {
    const messages: ChatMessages = [
      {
        role: "plan",
        message_id: "old-plan",
        content: "Archived plan",
        extra: { plan: { version: 99, mode: "agent" } },
      },
      {
        role: "event",
        subkind: "plan_delta",
        source: "test",
        message_id: "old-delta",
        content: "Archived delta",
        extra: {
          event: { subkind: "plan_delta", source: "test", payload: { seq: 1 } },
        },
      },
      {
        role: "compression_report",
        message_id: "report",
        content: "Rebuilt",
        compression_report: {
          kind: "reconstructed_history",
          schema_version: 1,
          payload: {
            messages: [
              {
                role: "plan",
                message_id: "current-plan",
                content: "Active plan",
                extra: { plan: { version: 1, mode: "agent" } },
              },
            ],
          },
        },
      },
      {
        role: "event",
        subkind: "plan_delta",
        source: "test",
        message_id: "new-delta",
        content: "Active update",
        extra: {
          event: { subkind: "plan_delta", source: "test", payload: { seq: 2 } },
        },
      },
    ];
    const state = makeState();
    const runtime = state.chat.threads["chat-1"];
    if (!runtime) throw new Error("Missing test thread");
    Object.assign(runtime.thread, { messages });
    const text = selectSynthesizedPlanText(state, "chat-1");
    expect(text).toContain("Active plan");
    expect(text).toContain("Active update");
    expect(text).not.toContain("Archived");
    expect(selectMessagesById(state, "chat-1")).toBe(messages);
  });
});
