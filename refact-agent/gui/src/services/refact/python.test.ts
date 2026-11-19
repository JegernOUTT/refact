import { describe, expect, it } from "vitest";
import {
  isPythonRunExtra,
  normalizePythonRunExtra,
  isPythonRunUpdate,
  isPythonRunSnapshot,
  isPythonApprovalDecisionKind,
} from "./types";
import { pythonCall } from "../../__tests__/pythonFixtures";

const extra = {
  schema: 1,
  run_id: "r",
  status: "completed",
  background: false,
  started_at_ms: 0,
  finished_at_ms: 1,
  workspace_dir: "",
  run_dir: "",
  calls: [pythonCall],
  calls_total: 1,
  calls_truncated: false,
  stdout_preview: "",
  stdout_bytes: 0,
  result_preview: "",
  result_bytes: 0,
  error: null,
  files_read: ["file"],
  files_written: [{ path: "file", bytes: 0, diffs: [] }],
  artifacts: [{ path: "file", bytes: 0 }],
  state: { restored: true, reset_reason: null, persisted: true, bytes: 0 },
};

describe("Python wire guards", () => {
  it("validates complete structured results including nested fields", () => {
    expect(isPythonRunExtra(extra)).toBe(true);
    expect(
      isPythonRunExtra({
        ...extra,
        error: { type: "ValueError", message: "bad", traceback: "trace" },
      }),
    ).toBe(true);
    for (const value of [
      null,
      [],
      {},
      { ...extra, schema: 2 },
      { ...extra, calls: [{}] },
      { ...extra, error: {} },
      { ...extra, state: {} },
      { ...extra, files_written: [{ path: "file", bytes: "1", diffs: [] }] },
    ]) {
      expect(isPythonRunExtra(value)).toBe(false);
    }
  });
  it("normalizes enum values from a newer engine instead of rejecting the run", () => {
    const newer = {
      ...extra,
      status: "paused_by_policy",
      calls: [
        { ...pythonCall, status: "throttled", approval: "delegated" },
        { ...pythonCall, call_id: "c2", seq: 2, status: "ok" },
      ],
    };
    const normalized = normalizePythonRunExtra(newer);
    expect(normalized?.status).toBe("unknown");
    expect(
      normalized?.calls.map((call) => [call.status, call.approval]),
    ).toEqual([
      ["unknown", "unknown"],
      ["ok", "not_needed"],
    ]);
    expect(isPythonRunExtra(newer)).toBe(true);
    expect(normalizePythonRunExtra({ ...extra, status: 3 })).toBeUndefined();
    expect(
      normalizePythonRunExtra({ ...extra, state: { ...extra.state, note: 4 } }),
    ).toBeUndefined();
  });
  it("reads the engine's null state notes as absent and keeps the written ones", () => {
    const nulls = normalizePythonRunExtra({
      ...extra,
      state: { ...extra.state, not_saved: null, note: null, notes: null },
    });
    expect(nulls?.state).toEqual(extra.state);
    const written = {
      not_saved:
        "not saved (disk full); variables come from the last successful save",
      note: "the session holds 18.0 MB of variables",
      notes: ["variables were not copied from chat a: it is running"],
    };
    expect(
      normalizePythonRunExtra({
        ...extra,
        state: { ...extra.state, ...written },
      })?.state,
    ).toEqual({ ...extra.state, ...written });
    for (const state of [
      { not_saved: 4 },
      { notes: "one note" },
      { notes: ["ok", 2] },
    ])
      expect(
        normalizePythonRunExtra({
          ...extra,
          state: { ...extra.state, ...state },
        }),
      ).toBeUndefined();
  });
  it("keeps a string error hint and read_only only when it is true", () => {
    const error = { type: "TypeError", message: "bad", traceback: "" };
    expect(
      normalizePythonRunExtra({
        ...extra,
        error: { ...error, hint: "Use x = None." },
      })?.error?.hint,
    ).toBe("Use x = None.");
    const mistyped = normalizePythonRunExtra({
      ...extra,
      error: { ...error, hint: 3 },
    });
    expect(mistyped?.error).toEqual(error);
    expect(
      normalizePythonRunExtra({ ...extra, read_only: true })?.read_only,
    ).toBe(true);
    for (const value of [false, "yes", 1]) {
      const normalized = normalizePythonRunExtra({
        ...extra,
        read_only: value,
      });
      expect(normalized).toBeDefined();
      expect(normalized && "read_only" in normalized).toBe(false);
    }
  });
  it("validates each update variant and snapshots", () => {
    for (const value of [
      { kind: "started", background: false },
      { kind: "call_started", record: pythonCall },
      { kind: "call_finished", record: pythonCall },
      { kind: "stdout", text: "hi" },
      { kind: "detached" },
      { kind: "finished", status: "completed" },
    ]) {
      expect(isPythonRunUpdate(value)).toBe(true);
    }
    const snapshot = {
      tool_call_id: "t",
      run_id: "r",
      status: "running",
      started_at_ms: 1,
      calls: [pythonCall],
      stdout_tail: "",
    };
    expect(isPythonRunSnapshot(snapshot)).toBe(true);
    expect(isPythonRunSnapshot({ ...snapshot, calls: [{}] })).toBe(false);
    expect(isPythonApprovalDecisionKind("stop")).toBe(true);
    expect(isPythonApprovalDecisionKind("allowed")).toBe(false);
  });
});
