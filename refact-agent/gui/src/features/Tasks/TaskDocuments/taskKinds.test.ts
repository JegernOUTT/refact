import { describe, expect, it } from "vitest";
import { DOCUMENT_KINDS } from "../../../services/refact/taskKinds";

describe("documentKindColor", () => {
  it("document_kind_helpers_export_known_kinds", () => {
    expect(DOCUMENT_KINDS).toContain("plan");
    expect(DOCUMENT_KINDS).toContain("design");
    expect(DOCUMENT_KINDS).toContain("runbook");
    expect(DOCUMENT_KINDS).toContain("brief");
    expect(DOCUMENT_KINDS).toContain("postmortem");
    expect(DOCUMENT_KINDS).toContain("spec");
    expect(DOCUMENT_KINDS).toHaveLength(6);
  });
});
