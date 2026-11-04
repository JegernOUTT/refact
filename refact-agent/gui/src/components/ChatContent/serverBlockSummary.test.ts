import { describe, expect, it } from "vitest";
import { summarizeServerBlocks } from "./serverBlockSummary";

describe("summarizeServerBlocks", () => {
  it("counts block types without dumping payloads", () => {
    const summary = summarizeServerBlocks([
      { type: "code_execution_result", secret: "x".repeat(500) },
      { type: "code_execution_result" },
      { foo: 1 },
    ]);
    expect(summary).toBe(
      "3 provider blocks: code_execution_result ×2, unknown",
    );
    expect(summary).not.toContain("xxx");
  });
});
