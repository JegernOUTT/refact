import { describe, expect, it } from "vitest";
import type { RootState } from "../../../app/store";
import {
  selectAutoCompactEnabled,
  selectAutoCompactEnabledById,
  selectAutoCompressionCap,
  selectAutoCompressionCapById,
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
