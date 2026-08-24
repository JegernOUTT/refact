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
          references: [
            { kind: "future", target: "x", provenance: "native" },
          ],
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
