import { describe, expect, it } from "vitest";
import type { RootState } from "../../../app/store";
import {
  selectAutoCompactEnabled,
  selectAutoCompactEnabledById,
} from "./selectors";

function makeState(autoCompactEnabled?: boolean): RootState {
  const thread =
    autoCompactEnabled === undefined
      ? {}
      : { auto_compact_enabled: autoCompactEnabled };

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
