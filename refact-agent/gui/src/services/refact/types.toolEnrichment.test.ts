import { describe, expect, it } from "vitest";
import { getToolEnrichment } from "./types";

describe("getToolEnrichment", () => {
  it("accepts the supported v1 envelope", () => {
    expect(
      getToolEnrichment({
        tool_enrichment: {
          schema_version: 1,
          references: [
            { kind: "path", target: "src/lib.rs", provenance: "native" },
            {
              kind: "query",
              target: "search term",
              provenance: "native",
              count: 3,
              source: "search_pattern",
            },
          ],
        },
      }),
    ).toMatchObject({ schema_version: 1 });
  });

  it("accepts bounded native Git, review, diff, and agent details", () => {
    expect(
      getToolEnrichment({
        tool_enrichment: {
          schema_version: 1,
          references: [
            {
              kind: "diff",
              target: "src/new.rs",
              provenance: "native",
              status: "applied",
              details: {
                action: "rename",
                rename_to: "src/new.rs",
                hunk_count: 2,
              },
            },
            {
              kind: "git",
              target: "workspace",
              provenance: "native",
              status: "available",
              details: { short_sha: "abc1234", scope: "card:T-54:stat:12" },
            },
            {
              kind: "review",
              target: "src/lib.rs",
              provenance: "native",
              status: "high",
              details: { action: "excerpt", line1: 10, line2: 12 },
            },
            {
              kind: "agent",
              target: "bgagent-1",
              provenance: "native",
              status: "completed",
              details: {
                parent_chat_id: "chat-parent",
                child_chat_id: "subchat-child",
                result_available: true,
                conflict: false,
              },
            },
          ],
        },
      }),
    ).toMatchObject({ schema_version: 1 });
  });

  it("ignores unknown versions, kinds, and oversized arrays", () => {
    expect(
      getToolEnrichment({
        tool_enrichment: { schema_version: 2, references: [] },
      }),
    ).toBeNull();
    expect(
      getToolEnrichment({
        tool_enrichment: {
          schema_version: 1,
          references: [{ kind: "future", target: "x", provenance: "native" }],
        },
      }),
    ).toBeNull();
    expect(
      getToolEnrichment({
        tool_enrichment: {
          schema_version: 1,
          references: Array.from({ length: 33 }, () => ({
            kind: "path",
            target: "src/lib.rs",
            provenance: "native",
          })),
        },
      }),
    ).toBeNull();
  });
});
