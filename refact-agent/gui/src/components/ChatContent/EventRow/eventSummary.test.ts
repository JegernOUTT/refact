import { describe, expect, it } from "vitest";
import type { EventMessage } from "../../../services/refact/types";
import { eventSummary } from "./eventSummary";
import { eventDetailFields, eventPayloadJson } from "./eventDetails";
import { eventTimestamp } from "./eventTimestamp";
const event = (
  subkind: string,
  payload: Record<string, unknown> = {},
): EventMessage => ({
  role: "event",
  subkind,
  source: "chat.session",
  content: " note\n text ",
  payload,
});
describe("event presentation", () => {
  it("summarizes real mode and process payloads", () => {
    expect(
      eventSummary(
        event("mode_switch", {
          from: "agent",
          to: "task_planner",
          reason: "new plan",
        }),
      ),
    ).toBe("Agent → Task Planner · new plan");
    expect(
      eventSummary(
        event("process_completed", {
          short_description: "npm test",
          exit_code: 0,
        }),
      ),
    ).toBe("npm test · exit 0");
  });
  it.each([
    "plan_delta",
    "goal_delta",
    "tick",
    "future_kind",
    "cron_fire",
    "cancellation_note",
  ])("collapses %s content", (kind) => {
    expect(eventSummary(event(kind))).toBe("note text");
  });
  it("flattens settings changes and delta truncation", () => {
    expect(
      eventDetailFields(
        event("mode_switch", {
          diff: { model: { from: "a", to: "b", changed: true } },
        }),
      ),
    ).toContainEqual({ label: "Settings", value: "model: a → b", mono: true });
    expect(
      eventDetailFields(
        event("plan_delta", {
          seq: 2,
          truncated: true,
          kept_chars: 10,
          original_chars: 20,
        }),
      ),
    ).toContainEqual({ label: "Truncated", value: "kept 10 of 20 chars" });
  });
  it("keeps only changed nodes of a real mode_switch diff", () => {
    const settings = eventDetailFields(
      event("mode_switch", {
        diff: {
          resolved: true,
          tools: {
            added: { count: 2, names: ["cat", "tree"], truncated: false },
            removed: { count: 0, names: [], truncated: false },
          },
          system_prompt: { changed: true },
          permissions: {
            allow_mcp: { from: true, to: false, changed: true },
            allow_subagents: { from: true, to: true, changed: false },
          },
          tool_confirm: {
            changed: false,
            from_rules_count: 2,
            to_rules_count: 2,
          },
        },
      }),
    ).find((field) => field.label === "Settings");

    expect(settings?.value.split("\n")).toEqual([
      "tools.added: 2 (cat, tree)",
      "system_prompt: changed",
      "permissions.allow_mcp: yes → no",
    ]);
  });

  it("omits payload keys the engine never emits", () => {
    expect(
      eventDetailFields(
        event("verifier_report", { kind: "review_prompt" }),
      ).map((field) => field.label),
    ).toEqual(["Source", "Kind", "Message", "Event kind"]);
  });

  it("omits empty payloads and missing or invalid timestamps", () => {
    expect(eventPayloadJson(event("tick"))).toBeNull();
    expect(eventTimestamp(event("tick"))).toBeNull();
    expect(eventTimestamp(event("tick", { timestamp: "invalid" }))).toBeNull();
    expect(
      eventTimestamp(event("tick", { timestamp: "2025-01-01T12:34:56" })),
    ).toBe("12:34:56");
  });
});
