import { describe, expect, test } from "vitest";
import { isCapsResponse } from "./caps";

const CAPS = {
  metadata: {},
  chat_default_model: "gpt-4o",
  chat_models: {},
};

describe("isCapsResponse", () => {
  test("accepts caps without legacy completion and embedding fields", () => {
    expect(isCapsResponse(CAPS)).toBe(true);
  });

  test("tolerates inert legacy completion and embedding fields", () => {
    expect(
      isCapsResponse({
        ...CAPS,
        completion_models: {},
        completion_default_model: "",
        code_completion_n_ctx: 0,
        embedding_model: { name: "", type: "embedding" },
      }),
    ).toBe(true);
  });

  test("rejects caps without chat models", () => {
    expect(isCapsResponse({ metadata: {}, chat_default_model: "gpt-4o" })).toBe(
      false,
    );
  });
});
