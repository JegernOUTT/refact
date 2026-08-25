import { describe, expect, it } from "vitest";
import { getToolEnrichment, getToolEnrichmentPathReferences } from "./types";

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
              line1: 10,
              line2: 12,
              details: { action: "excerpt" },
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

  it("reads only safe legacy exec path enrichment during migration", () => {
    expect(
      getToolEnrichmentPathReferences({
        path_enrichment: {
          schema_version: 1,
          references: [
            { path: "src/lib.rs", line1: 3, line2: 5 },
            { path: "../secret.rs", line1: 1 },
          ],
        },
      }),
    ).toEqual([
      {
        kind: "path",
        target: "src/lib.rs",
        provenance: "heuristic",
        line1: 3,
        line2: 5,
        source: undefined,
      },
    ]);
  });

  it("rejects unsafe unified paths and hides restricted envelopes", () => {
    expect(
      getToolEnrichmentPathReferences({
        tool_enrichment: {
          schema_version: 1,
          references: [
            { kind: "path", target: "../secret.rs", provenance: "native" },
            { kind: "path", target: "src/lib.rs", provenance: "native" },
          ],
        },
      }),
    ).toEqual([{ kind: "path", target: "src/lib.rs", provenance: "native" }]);
    expect(
      getToolEnrichmentPathReferences({
        tool_enrichment: {
          schema_version: 1,
          references: [
            { kind: "path", target: "src/lib.rs", provenance: "native" },
          ],
          privacy: { redacted: true, restricted: true },
        },
      }),
    ).toEqual([]);
  });
});
