import { describe, expect, it } from "vitest";
import { extractProvider } from "./modelProviders";
import { enrichAndGroupModels } from "./enrichModels";

describe("modelProviders", () => {
  it("groups provider-qualified ids by prefix before vendor heuristics", () => {
    expect(extractProvider("custom/gpt-4o-mini")).toBe("custom");
    expect(extractProvider("openai_codex/gpt-5.5")).toBe("openai_codex");
    expect(extractProvider("local/claude-ish")).toBe("local");
  });

  it("keeps local provider ids in their own enriched group", () => {
    const groups = enrichAndGroupModels(
      [
        {
          value: "custom/gpt-4o-mini",
          textValue: "custom/gpt-4o-mini",
          disabled: false,
        },
        {
          value: "local/claude-ish",
          textValue: "local/claude-ish",
          disabled: false,
        },
      ],
      undefined,
    );

    expect(groups.map((group) => group.provider)).toEqual(["custom", "local"]);
  });
});
