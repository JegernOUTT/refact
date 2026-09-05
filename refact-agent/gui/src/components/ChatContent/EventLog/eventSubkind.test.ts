import { describe, expect, it } from "vitest";
import {
  EVENT_SUBKINDS,
  eventSubkindIcon,
  eventSubkindLabel,
  eventTone,
} from "./eventSubkind";
describe("event presentation", () => {
  it("covers all 13 event kinds with human labels and icons", () => {
    expect(EVENT_SUBKINDS).toHaveLength(13);
    for (const kind of EVENT_SUBKINDS) {
      expect(eventSubkindIcon(kind)).toBeDefined();
      expect(eventSubkindLabel(kind)).not.toContain("_");
    }
  });
  it.each([
    ["process_completed", { exit_code: 1 }, "danger"],
    ["process_completed", { exit_code: 0 }, "success"],
    ["system_notice", {}, "default"],
    ["system_notice", { status: "failed" }, "danger"],
    ["verifier_report", { verdict: "needs_work" }, "warning"],
    ["future_kind", {}, "muted"],
  ])("refines %s by payload %j", (kind, payload, tone) =>
    expect(eventTone(kind, payload)).toBe(tone),
  );
  it("provides stable unknown fallback", () => {
    expect({
      label: eventSubkindLabel("future_kind"),
      tone: eventTone("future_kind", {}),
    }).toMatchInlineSnapshot(`
      {
        "label": "Future kind",
        "tone": "muted",
      }
    `);
    expect(eventSubkindIcon("future_kind")).toBeDefined();
  });
});
