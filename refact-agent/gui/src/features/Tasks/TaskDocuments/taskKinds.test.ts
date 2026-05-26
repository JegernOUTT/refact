import { describe, expect, it } from "vitest";
import {
  documentKindColor,
  DOCUMENT_KINDS,
} from "../../../services/refact/taskKinds";

describe("documentKindColor", () => {
  it("unknown_document_kind_renders_gray_badge", () => {
    expect(documentKindColor("roadmap")).toBe("gray");
    expect(documentKindColor("")).toBe("gray");
    expect(documentKindColor("sprint")).toBe("gray");
  });

  it("document_kind_helpers_export_known_kinds", () => {
    expect(DOCUMENT_KINDS).toContain("plan");
    expect(DOCUMENT_KINDS).toContain("design");
    expect(DOCUMENT_KINDS).toContain("runbook");
    expect(DOCUMENT_KINDS).toContain("brief");
    expect(DOCUMENT_KINDS).toContain("postmortem");
    expect(DOCUMENT_KINDS).toContain("spec");
    expect(DOCUMENT_KINDS).toHaveLength(6);
  });

  it("returns correct color for known kinds", () => {
    expect(documentKindColor("plan")).toBe("blue");
    expect(documentKindColor("design")).toBe("purple");
    expect(documentKindColor("postmortem")).toBe("red");
  });
});
