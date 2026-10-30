import { describe, expect, it } from "vitest";
import {
  canDecide,
  clampGeometry,
  sameRequest,
  visibleLiteral,
} from "./companionPolicy";
import {
  isAttentionSnapshot,
  isDecisionReceipt,
  type AttentionRow,
} from "../../../services/refact/buddyCompanion";

export const requestFixture: AttentionRow = {
  id: "request-a",
  kind: "tool_permission",
  chat_id: "source-a",
  source_version: "v1",
  reason: "Permission required",
  effect: "execute_pending_tool",
  actionable: true,
  literal_command: "echo hello",
  display_command: "shell echo hello",
  cwd: null,
  tool_call_id: "tool-a",
  question: null,
  parent_chat_id: null,
  run_generation: null,
  expires_at: null,
  tool_name: "shell",
  tool_arguments: { command: "echo hello" },
};
describe("Buddy companion safety policy", () => {
  it("allows exact native tools with visibly unknown cwd, never cache/browser/policy/child approval", () => {
    expect(canDecide(requestFixture)).toBe(true);
    for (const effect of [
      "cache_extra_cost",
      "browser_context_release",
      "unknown",
    ])
      expect(canDecide({ ...requestFixture, effect })).toBe(false);
    expect(canDecide({ ...requestFixture, kind: "child_permission" })).toBe(
      false,
    );
    expect(canDecide({ ...requestFixture, actionable: false })).toBe(false);
    expect(canDecide({ ...requestFixture, tool_arguments: null })).toBe(false);
  });
  it("does not rebind a selected request to a newer source guard", () => {
    expect(
      sameRequest(requestFixture, { ...requestFixture, source_version: "v2" }),
    ).toBe(false);
    expect(
      sameRequest(requestFixture, { ...requestFixture, id: "new-request" }),
    ).toBe(false);
    expect(sameRequest(requestFixture, { ...requestFixture })).toBe(true);
    const question = {
      ...requestFixture,
      kind: "question",
      question: {
        tool_call_id: "q",
        asked_at_ms: 1,
        status: "pending" as const,
        questions: [
          { id: "one", type: "free_text" as const, text: "Original question" },
        ],
      },
    };
    expect(
      sameRequest(question, {
        ...question,
        question: { ...question.question, asked_at_ms: 2 },
      }),
    ).toBe(false);
    expect(
      sameRequest(question, {
        ...question,
        question: {
          ...question.question,
          questions: [
            { ...question.question.questions[0], text: "Changed question" },
          ],
        },
      }),
    ).toBe(false);
  });
  it("makes bidirectional and command control characters inspectable", () => {
    expect(visibleLiteral("echo \u202esecret\u0007")).toBe(
      "echo \\u202esecret\\u0007",
    );
  });
  it("clamps resized and offscreen windows to a narrow keyboard viewport", () => {
    const result = clampGeometry(
      { x: 1800, y: -500, width: 900, height: 800 },
      320,
      300,
    );
    expect(result).toEqual({ x: 12, y: 12, width: 296, height: 276 });
  });
  it("rejects malformed attention and false receipt success before rendering actions", () => {
    const snapshot = {
      schema_version: 1,
      project_root: "/project",
      engine_identity: "engine",
      coverage: "loaded_sessions",
      partial: true,
      busy_chat_ids: [],
      rows: [requestFixture],
    };
    expect(isAttentionSnapshot(snapshot)).toBe(true);
    expect(
      isAttentionSnapshot({
        ...snapshot,
        rows: [{ ...requestFixture, actionable: "yes" }],
      }),
    ).toBe(false);
    expect(isAttentionSnapshot({ ...snapshot, schema_version: 2 })).toBe(false);
    const receipt = {
      project_root: "/project",
      engine_identity: "engine",
      chat_id: "source-a",
      client_request_id: "intent",
      request_identity: "request-a",
      source_version: "v1",
      payload_hash: "hash",
      target_id: "tool-a",
      accepted: true,
      status: "queued",
      followup: "pending",
    };
    expect(isDecisionReceipt(receipt)).toBe(true);
    expect(isDecisionReceipt({ ...receipt, status: "success" })).toBe(false);
    expect(isDecisionReceipt({ ...receipt, followup: "done" })).toBe(false);
  });
});
