import { describe, expect, it } from "vitest";
import {
  dropInvalidOptionals,
  isPythonWorkspace,
  pythonWorkspaceApi,
} from "./pythonWorkspace";
import { setUpStore } from "../../app/store";
import { server } from "../../utils/mockServer";
import { http, HttpResponse } from "msw";

import { workspaceExample } from "./pythonWorkspace.fixture";
describe("Python workspace guard", () => {
  it("accepts and caps oversized lists with truncation flags", async () => {
    const oversized = {
      ...workspaceExample,
      session: {
        ...workspaceExample.session,
        variables: Array.from(
          { length: 501 },
          () => workspaceExample.session.variables[0],
        ),
      },
      artifacts: Array.from(
        { length: 501 },
        () => workspaceExample.artifacts[0],
      ),
      runs: Array.from({ length: 501 }, () => workspaceExample.runs[0]),
    };
    expect(isPythonWorkspace(oversized)).toBe(true);
    server.use(
      http.get("*/v1/python/workspace", () => HttpResponse.json(oversized)),
    );
    const result = await setUpStore()
      .dispatch(
        pythonWorkspaceApi.endpoints.getPythonWorkspace.initiate({
          chatId: "chat-a",
        }),
      )
      .unwrap();
    expect(result.session.variables).toHaveLength(500);
    expect(result.artifacts).toHaveLength(500);
    expect(result.runs).toHaveLength(500);
    expect(result.session.variables_truncated).toBe(true);
    expect(result.artifacts_truncated).toBe(true);
    expect(result.runs_truncated).toBe(true);
    expect(oversized.runs).toHaveLength(501);
  });
  it("accepts unfinished runs with null finish time", () =>
    expect(
      isPythonWorkspace({
        ...workspaceExample,
        runs: [{ ...workspaceExample.runs[0], finished_at_ms: null }],
      }),
    ).toBe(true));
  it("accepts the contract", () =>
    expect(isPythonWorkspace(workspaceExample)).toBe(true));
  it.each([
    null,
    {},
    { ...workspaceExample, schema: 2 },
    { ...workspaceExample, session: { ...workspaceExample.session, live: 1 } },
    { ...workspaceExample, artifacts: [{ path: "x" }] },
    { ...workspaceExample, disk: { workspace_bytes: -1, quota_bytes: 0 } },
  ])("rejects malformed responses", (value) =>
    expect(isPythonWorkspace(value)).toBe(false),
  );
  it("drops invalid optional fields instead of rejecting the workspace", () => {
    const value = dropInvalidOptionals({
      ...workspaceExample,
      session: {
        ...workspaceExample.session,
        variables_total: null,
        last_run_restored: "yes",
      },
      runs: [
        { ...workspaceExample.runs[0], description: null, code_preview: 3 },
      ],
    });
    expect(isPythonWorkspace(value)).toBe(true);
    const workspace = value as typeof workspaceExample & {
      runs: Record<string, unknown>[];
    };
    expect("description" in workspace.runs[0]).toBe(false);
    expect("variables_total" in workspace.session).toBe(false);
    expect(
      isPythonWorkspace(
        dropInvalidOptionals({
          ...workspaceExample,
          runs: [{ ...workspaceExample.runs[0], description: "ok" }],
        }),
      ),
    ).toBe(true);
  });
  it("drops a null or invalid description and code preview, and every new optional field", () => {
    const display = {
      index: 1,
      kind: "table",
      truncated: false,
      path: "/d.json",
      run_id: "run-one",
      started_at_ms: 5,
    };
    const value = dropInvalidOptionals({
      ...workspaceExample,
      runs: [
        {
          ...workspaceExample.runs[0],
          description: null,
          code_preview: null,
          llm_calls: -1,
          displays_total: "3",
          engine_changes: null,
        },
      ],
      displays: [display, { ...display, run_id: 3 }, { kind: "bar" }],
      displays_truncated: null,
    }) as Record<string, unknown> & {
      runs: Record<string, unknown>[];
      displays: unknown[];
    };
    expect(isPythonWorkspace(value)).toBe(true);
    for (const key of [
      "description",
      "code_preview",
      "llm_calls",
      "displays_total",
      "engine_changes",
    ])
      expect(key in value.runs[0]).toBe(false);
    expect(value.displays).toHaveLength(1);
    expect("displays_truncated" in value).toBe(false);
    expect(
      isPythonWorkspace(
        dropInvalidOptionals({ ...workspaceExample, displays: "x" }),
      ),
    ).toBe(true);
  });
  it("keeps read_only and variables_unknown only as booleans", () => {
    const valid = {
      ...workspaceExample,
      session: { ...workspaceExample.session, variables_unknown: true },
      runs: [{ ...workspaceExample.runs[0], read_only: true }],
    };
    expect(isPythonWorkspace(dropInvalidOptionals(valid))).toBe(true);
    expect(dropInvalidOptionals(valid)).toEqual(valid);
    const invalid = dropInvalidOptionals({
      ...workspaceExample,
      session: { ...workspaceExample.session, variables_unknown: "yes" },
      runs: [{ ...workspaceExample.runs[0], read_only: 1 }],
    }) as typeof workspaceExample & {
      runs: Record<string, unknown>[];
      session: Record<string, unknown>;
    };
    expect(isPythonWorkspace(invalid)).toBe(true);
    expect("read_only" in invalid.runs[0]).toBe(false);
    expect("variables_unknown" in invalid.session).toBe(false);
    expect(
      isPythonWorkspace({
        ...workspaceExample,
        runs: [{ ...workspaceExample.runs[0], read_only: "yes" }],
      }),
    ).toBe(false);
  });
  it("keeps the newest eight displays", async () => {
    const displays = Array.from({ length: 10 }, (_, index) => ({
      index: 1,
      kind: "table",
      truncated: false,
      path: `/d${index}.json`,
      run_id: `r${index}`,
      started_at_ms: index,
    }));
    server.use(
      http.get("*/v1/python/workspace", () =>
        HttpResponse.json({ ...workspaceExample, displays }),
      ),
    );
    const result = await setUpStore()
      .dispatch(
        pythonWorkspaceApi.endpoints.getPythonWorkspace.initiate({
          chatId: "chat-a",
        }),
      )
      .unwrap();
    expect(result.displays?.map((item) => item.run_id)).toEqual([
      "r9",
      "r8",
      "r7",
      "r6",
      "r5",
      "r4",
      "r3",
      "r2",
    ]);
    expect(result.displays_truncated).toBe(true);
  });
});
