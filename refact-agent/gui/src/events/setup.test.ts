import { describe, expect, it } from "vitest";
import { EVENT_NAMES_FROM_SETUP, isOpenExternalUrl } from "./setup";

describe("setup events", () => {
  it("recognizes open external URL events with string URLs", () => {
    expect(
      isOpenExternalUrl({
        type: EVENT_NAMES_FROM_SETUP.OPEN_EXTERNAL_URL,
        payload: { url: "http://127.0.0.1:8001" },
      }),
    ).toBe(true);
  });

  it("rejects malformed open external URL events", () => {
    expect(
      isOpenExternalUrl({
        type: EVENT_NAMES_FROM_SETUP.OPEN_EXTERNAL_URL,
        payload: {},
      }),
    ).toBe(false);
    expect(
      isOpenExternalUrl({
        type: EVENT_NAMES_FROM_SETUP.OPEN_EXTERNAL_URL,
        payload: { url: 8001 },
      }),
    ).toBe(false);
    expect(isOpenExternalUrl({ type: "other", payload: { url: "x" } })).toBe(
      false,
    );
  });
});
