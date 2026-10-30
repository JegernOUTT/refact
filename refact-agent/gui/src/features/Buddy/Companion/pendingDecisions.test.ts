import { beforeEach, expect, it } from "vitest";
import {
  persistPendingDecision,
  readPendingDecisions,
  restoredDecisionRow,
} from "./pendingDecisions";
import type { AttentionRow } from "../../../services/refact/buddyCompanion";

beforeEach(() => localStorage.clear());
it("restores only decision identity metadata and refuses a new identity for the same request", () => {
  const row: AttentionRow = {
    id: "row",
    chat_id: "source",
    tool_call_id: "tool",
    source_version: "v1",
    kind: "tool_permission",
    reason: "private reason",
    effect: "execute_pending_tool",
    actionable: true,
    literal_command: "secret command",
    display_command: "secret",
    cwd: "/secret",
    question: null,
    parent_chat_id: null,
    run_generation: null,
    expires_at: null,
  };
  const intent = {
    id: "stable-id",
    engine: "original-engine",
    accepted: false,
  };
  persistPendingDecision("host-project", "/project", row, intent);
  const records = readPendingDecisions("host-project");
  expect(records).toHaveLength(1);
  expect(records[0].intent).toEqual(intent);
  expect(JSON.stringify(records)).not.toMatch(
    /secret|private reason|apiKey|draft/,
  );
  expect(readPendingDecisions("other-project")).toEqual([]);
  expect(() =>
    persistPendingDecision("host-project", "/project", row, {
      ...intent,
      id: "new-id",
    }),
  ).toThrow("reconciled first");
  const restored = restoredDecisionRow(records[0]);
  expect(restored.actionable).toBe(false);
  expect(restored.tool_call_id).toBe("tool");
  expect(restored.literal_command).toBeNull();
});
it("ignores malformed persisted records instead of authorizing or replaying them", () => {
  localStorage.setItem(
    "refact:buddy:pending-decisions:v1:project",
    JSON.stringify([{ rowId: "row", intent: { id: "id", accepted: "yes" } }]),
  );
  expect(readPendingDecisions("project")).toEqual([]);
});
