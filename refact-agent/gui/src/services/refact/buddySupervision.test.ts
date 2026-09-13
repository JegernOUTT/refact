import { describe, expect, it } from "vitest";
import { http, HttpResponse } from "msw";
import { server } from "../../utils/mockServer";
import { setUpStore } from "../../app/store";
import { buddyOperatorIdentity } from "./buddyOperator";
import {
  buildSupervisionGrant,
  buildGoalIntake,
  buildControl,
  buildSteeringNotes,
  buddySupervisionApi,
  MAX_CONTROL_HISTORY,
  type SupervisionContext,
} from "./buddySupervision";

import {
  parseSupervisionContext,
  parseSupervisionStatus,
} from "./buddySupervision";
const project = { project_id: "p", canonical_root: "/fixture" };
const args = {
  endpoint_identity: "endpoint",
  project,
  worker: { worker_id: "w", engine_instance_id: "e", epoch: 1 },
  id: "m",
};
const link = {
  schema_version: 1,
  project,
  mandate_id: "m",
  chat_id: "c",
  command_acknowledged: true,
  dispatch_attempted: false,
  revoked: false,
  escalation: null,
  delivery_run_id: null,
};
describe("supervision snapshots", () => {
  it("keeps omitted native observations unknown and rejects oversized control history", () => {
    const raw = {
      project,
      worker: args.worker,
      link,
      coverage: "native only",
      host_wake_supported: false,
      daemon_wake_supported: false,
    };
    const parsed = parseSupervisionStatus(raw, args);
    expect(parsed.native_goal).toBeNull();
    expect(parsed.native_worker).toBeNull();
    expect(parsed.native_worker_goal).toBeNull();
    expect(parsed.native_pause).toBeNull();
    const controls = Array.from({ length: 32 }, (_, index) => ({
      control_id: `pause-${index}`,
      kind: "pause",
      note_id: null,
      requested_by: {
        source_kind: "external",
        source_id: "user",
        source_version: "1",
      },
      at_ms: index,
      outcome: "dispatch_outcome_unknown",
    }));
    expect(
      parseSupervisionStatus(
        { ...raw, link: { ...link, control_history: controls } },
        args,
      ).link.control_history,
    ).toHaveLength(32);
    expect(() =>
      parseSupervisionStatus(
        {
          ...raw,
          link: { ...link, control_history: [...controls, controls[0]] },
        },
        args,
      ),
    ).toThrow();
  });

  it("preserves disabled rollout without manufacturing authority", () => {
    const parsed = parseSupervisionContext({
      schema_version: 1,
      project: null,
      worker: null,
      authenticated_owner: null,
      features: {
        storage: false,
        actions: false,
        editorial: false,
        supervision: false,
        improvement: false,
      },
    });
    expect(parsed.features.supervision).toBe(false);
    expect(parsed.project).toBeNull();
    expect(parsed.authenticated_owner).toBeNull();
    expect(() =>
      parseSupervisionContext({ ...parsed, features: {} }),
    ).toThrow();
  });
  it("preserves missing native observations as unknown, not completion", () => {
    const raw = {
      project,
      worker: args.worker,
      link,
      coverage: "native only",
      host_wake_supported: false,
      daemon_wake_supported: false,
    };
    const parsed = parseSupervisionStatus(raw, args);
    expect(parsed.native_goal).toBeNull();
    for (const worker of [
      null,
      { ...args.worker, epoch: 2 },
      { ...args.worker, engine_instance_id: "other" },
      { ...args.worker, worker_id: "other" },
    ])
      expect(() => parseSupervisionStatus({ ...raw, worker }, args)).toThrow();
    expect(parsed.native_worker).toBeNull();
    expect(parsed.native_worker_goal).toBeNull();
    expect(parsed.native_pause).toBeNull();
    expect(parsed.link.delivery_run_id).toBeNull();
    expect(parsed.link.command_acknowledged).toBe(true);
    expect(() =>
      parseSupervisionStatus({ ...raw, host_wake_supported: true }, args),
    ).toThrow();
    expect(() =>
      parseSupervisionStatus(
        {
          ...raw,
          link: { ...link, project: { ...project, canonical_root: "/other" } },
        },
        args,
      ),
    ).toThrow();
    expect(() =>
      parseSupervisionStatus(
        { ...raw, link: { ...link, mandate_id: "other" } },
        args,
      ),
    ).toThrow();
    expect(() =>
      parseSupervisionStatus(
        {
          ...raw,
          link: {
            ...link,
            control_history: Array.from({
              length: MAX_CONTROL_HISTORY + 1,
            }),
          },
        },
        args,
      ),
    ).toThrow();
  });
});

const context: SupervisionContext = {
  schema_version: 1,
  project,
  worker: args.worker,
  authenticated_owner: {
    source_kind: "external",
    source_id: "authenticated-user",
    source_version: "operator-http-v1",
  },
  features: {
    storage: true,
    actions: false,
    editorial: false,
    supervision: true,
    improvement: false,
  },
};
const expiry = () => new Date(Date.now() + 3_600_000).toISOString();
const grant = () =>
  buildSupervisionGrant(
    context,
    "m",
    "provider/exact-model",
    10,
    20000,
    10,
    2,
    expiry(),
  );

describe("bounded supervision builders", () => {
  it("copies only explicit scope, authenticated owner, exact model and finite budgets", () => {
    const request = grant();
    expect(request.max_workers).toBe(1);
    expect(request.routine_choices).toEqual([]);
    expect(request.mandate).toEqual({
      schema_version: 1,
      mandate_id: "m",
      project,
      owner: context.authenticated_owner,
      owner_epoch: 1,
      resource_scopes: [
        { resource_kind: "project", resource_id: "p", version: "1" },
      ],
      allowed_operations: ["create_goal", "spawn_worker"],
      allowed_authority_classes: ["read_only"],
      model_constraints: ["provider/exact-model"],
      budget_limit: {
        max_turns: 10,
        max_tokens: 20000,
        max_duration_seconds: 600,
        max_cost_micros: 20000,
      },
      expires_at: request.mandate.expires_at,
      revoked_at: null,
    });
    expect(request.mandate.expires_at).toEqual(expect.any(String));
  });
  it.each([0, -1, NaN, Infinity, 1.5, 101])(
    "rejects non-finite or out-of-range turns %s",
    (turns) => {
      expect(() =>
        buildSupervisionGrant(
          context,
          "m",
          "model",
          turns,
          20000,
          10,
          null,
          expiry(),
        ),
      ).toThrow();
    },
  );
  it.each([0, -1, NaN, Infinity, 1.5, 1_000_001])(
    "rejects invalid tokens %s",
    (tokens) => {
      expect(() =>
        buildSupervisionGrant(
          context,
          "m",
          "model",
          10,
          tokens,
          10,
          null,
          expiry(),
        ),
      ).toThrow();
    },
  );
  it.each([0, -1, NaN, Infinity, 1.5, 1441])(
    "rejects invalid whole minutes %s",
    (minutes) => {
      expect(() =>
        buildSupervisionGrant(
          context,
          "m",
          "model",
          10,
          20000,
          minutes,
          null,
          expiry(),
        ),
      ).toThrow();
    },
  );
  it.each([0, -1, NaN, Infinity, 1.5, Number.MAX_SAFE_INTEGER])(
    "rejects inexact cent cost %s",
    (cost) => {
      expect(() =>
        buildSupervisionGrant(
          context,
          "m",
          "model",
          10,
          20000,
          10,
          cost,
          expiry(),
        ),
      ).toThrow();
    },
  );
  it("rejects unavailable authority, empty models and expired or unbounded grants", () => {
    for (const invalid of [
      { ...context, project: null },
      { ...context, worker: null },
      { ...context, authenticated_owner: null },
      { ...context, features: { ...context.features, supervision: false } },
    ]) {
      expect(() =>
        buildSupervisionGrant(
          invalid,
          "m",
          "model",
          10,
          100,
          10,
          null,
          expiry(),
        ),
      ).toThrow();
    }
    expect(() =>
      buildSupervisionGrant(context, "m", " ", 10, 100, 10, null, expiry()),
    ).toThrow();
    for (const expires of [
      "",
      "not-a-date",
      new Date(Date.now() - 1).toISOString(),
      new Date(Date.now() + 90_000_000).toISOString(),
    ]) {
      expect(() =>
        buildSupervisionGrant(
          context,
          "m",
          "model",
          10,
          100,
          10,
          null,
          expires,
        ),
      ).toThrow();
    }
  });
  it("builds explicit criteria and enforces UTF-8 limits without inventing success", () => {
    expect(
      buildGoalIntake("Objective", " First proof\n\nSecond proof "),
    ).toEqual({
      content: "Objective",
      criteria: [
        { id: "C1", text: "First proof", verify_hint: null },
        { id: "C2", text: "Second proof", verify_hint: null },
      ],
    });
    expect(() => buildGoalIntake("Objective", " ")).toThrow();
    expect(() =>
      buildGoalIntake(
        "Objective",
        Array.from({ length: 13 }, () => "criterion").join("\n"),
      ),
    ).toThrow();
    expect(() => buildGoalIntake("😀".repeat(4001), "proof")).toThrow();
    expect(() => buildGoalIntake("Objective", "😀".repeat(501))).toThrow();
  });
  it("keeps tool-free grants closed and bounds engineering tools, files, and notes", () => {
    expect(grant()).toMatchObject({
      worker_tools: [],
      worker_target_files: [],
      steering_notes: [],
    });
    const engineering = buildSupervisionGrant(
      context,
      "m",
      "model",
      1,
      1,
      1,
      null,
      expiry(),
      {
        mode: "engineering",
        tools: ["apply_patch"],
        target_files: ["src/only.ts"],
      },
      buildSteeringNotes("Finish the bounded task"),
    );
    expect(engineering.worker_tools).toEqual(["apply_patch"]);
    expect(engineering.worker_target_files).toEqual(["src/only.ts"]);
    expect(engineering.mandate.allowed_operations).toEqual([
      "create_goal",
      "spawn_worker",
      "read_project",
      "engineer_project",
      "steer_worker",
    ]);
    expect(() =>
      buildSupervisionGrant(context, "m", "model", 1, 1, 1, null, expiry(), {
        mode: "engineering",
        tools: ["shell"],
        target_files: ["../escape"],
      }),
    ).toThrow();
    expect(() => buildControl("message", "c", " ")).toThrow();
    expect(() => buildControl("pause", "c", "N1")).toThrow();
    expect(() => buildSteeringNotes("x\n".repeat(13))).toThrow();
  });
  it("keeps read_project tool declarations empty because native adds its fixed reads", () => {
    const readOnly = buildSupervisionGrant(
      context,
      "m",
      "model",
      1,
      1,
      1,
      null,
      expiry(),
      { mode: "read_only" },
    );
    expect(readOnly.worker_tools).toEqual([]);
    expect(readOnly.worker_target_files).toEqual([]);
    expect(readOnly.mandate.allowed_operations).toEqual([
      "create_goal",
      "spawn_worker",
      "read_project",
    ]);
  });
});

describe("supervision transport binding", () => {
  it("rejects a create acknowledgement that expands the requested scope", async () => {
    const store = setUpStore({
      config: { host: "vscode", lspPort: 8001, themeProps: {} },
    });
    const request = grant();
    server.use(
      http.post("*/v1/buddy/supervision/mandates", () =>
        HttpResponse.json({
          revision: 1,
          change_seq: 1,
          value: {
            schema_version: 1,
            project,
            worker: args.worker,
            mandate: request.mandate,
            worker_tools: [],
            worker_target_files: [],
            steering_notes: [{ note_id: "new-note", text: "Not authorized" }],
          },
        }),
      ),
    );
    await expect(
      store
        .dispatch(
          buddySupervisionApi.endpoints.changeBuddySupervision.initiate({
            ...args,
            endpoint_identity: buddyOperatorIdentity(store.getState()),
            operation: "create",
            request,
          }),
        )
        .unwrap(),
    ).rejects.toMatchObject({ status: "CUSTOM_ERROR" });
    store.dispatch(buddySupervisionApi.util.resetApiState());
  });

  it("posts exact native control JSON to the control endpoints", async () => {
    const store = setUpStore({
      config: { host: "vscode", lspPort: 8001, themeProps: {} },
    });
    const received: { path: string; body: unknown }[] = [];
    const bound = {
      ...args,
      endpoint_identity: buddyOperatorIdentity(store.getState()),
    };
    server.use(
      http.post(
        "*/v1/buddy/supervision/mandates/m/:operation",
        async ({ request }) => {
          received.push({
            path: new URL(request.url).pathname,
            body: await request.json(),
          });
          return HttpResponse.json({ ...link, worker: args.worker });
        },
      ),
    );
    for (const control of [
      {
        operation: "message",
        request: { control_id: "message-N1", note_id: "N1" },
      },
      { operation: "pause", request: { control_id: "pause" } },
      { operation: "resume", request: { control_id: "resume" } },
      { operation: "cancel", request: { control_id: "cancel" } },
    ] as const)
      await store
        .dispatch(
          buddySupervisionApi.endpoints.changeBuddySupervision.initiate({
            ...bound,
            ...control,
          }),
        )
        .unwrap();
    expect(received).toEqual([
      {
        path: "/v1/buddy/supervision/mandates/m/message",
        body: { control_id: "message-N1", note_id: "N1" },
      },
      {
        path: "/v1/buddy/supervision/mandates/m/pause",
        body: { control_id: "pause" },
      },
      {
        path: "/v1/buddy/supervision/mandates/m/resume",
        body: { control_id: "resume" },
      },
      {
        path: "/v1/buddy/supervision/mandates/m/cancel",
        body: { control_id: "cancel" },
      },
    ]);
    store.dispatch(buddySupervisionApi.util.resetApiState());
  });
  it("sends all four identity headers for status and every supported mutation", async () => {
    const store = setUpStore({
      config: { host: "vscode", lspPort: 8001, themeProps: {} },
    });
    const identityArgs = {
      ...args,
      endpoint_identity: buddyOperatorIdentity(store.getState()),
    };
    const requests: string[] = [];
    const check = (request: Request) => {
      requests.push(new URL(request.url).pathname);
      expect(request.headers.get("X-Refact-Project-Id")).toBe("p");
      expect(request.headers.get("X-Refact-Worker-Id")).toBe("w");
      expect(request.headers.get("X-Refact-Engine-Instance")).toBe("e");
      expect(request.headers.get("X-Refact-Worker-Epoch")).toBe("1");
    };
    const request = grant();
    server.use(
      http.post("*/v1/buddy/supervision/mandates", ({ request: incoming }) => {
        check(incoming);
        return HttpResponse.json({
          revision: 1,
          change_seq: 1,
          value: {
            schema_version: 1,
            project,
            worker: args.worker,
            mandate: request.mandate,
          },
        });
      }),
      http.post(
        "*/v1/buddy/supervision/mandates/m/:operation",
        ({ request: incoming }) => {
          check(incoming);
          return HttpResponse.json({ ...link, worker: args.worker });
        },
      ),
      http.get(
        "*/v1/buddy/supervision/mandates/m/status",
        ({ request: incoming }) => {
          check(incoming);
          return HttpResponse.json({
            project,
            link,
            worker: args.worker,
            native_goal: null,
            native_worker: null,
            native_worker_goal: null,
            coverage: "native only",
            host_wake_supported: false,
            daemon_wake_supported: false,
          });
        },
      ),
    );
    await store
      .dispatch(
        buddySupervisionApi.endpoints.changeBuddySupervision.initiate({
          ...identityArgs,
          operation: "create",
          request,
        }),
      )
      .unwrap();
    await store
      .dispatch(
        buddySupervisionApi.endpoints.changeBuddySupervision.initiate({
          ...identityArgs,
          operation: "intake",
          request: buildGoalIntake("Objective", "Proof"),
        }),
      )
      .unwrap();
    for (const operation of ["spawn", "reconcile", "revoke"] as const) {
      await store
        .dispatch(
          buddySupervisionApi.endpoints.changeBuddySupervision.initiate({
            ...identityArgs,
            operation,
          }),
        )
        .unwrap();
    }
    const status = store.dispatch(
      buddySupervisionApi.endpoints.getBuddySupervisionStatus.initiate(
        identityArgs,
      ),
    );
    await status.unwrap();
    status.unsubscribe();
    expect(requests).toHaveLength(6);
    store.dispatch(buddySupervisionApi.util.resetApiState());
  });
  it("rejects a mandate acknowledgement from another worker", async () => {
    const store = setUpStore({
      config: { host: "vscode", lspPort: 8001, themeProps: {} },
    });
    const request = grant();
    server.use(
      http.post("*/v1/buddy/supervision/mandates", () =>
        HttpResponse.json({
          revision: 1,
          change_seq: 1,
          value: {
            schema_version: 1,
            project,
            worker: { ...args.worker, epoch: 2 },
            mandate: request.mandate,
          },
        }),
      ),
    );
    await expect(
      store
        .dispatch(
          buddySupervisionApi.endpoints.changeBuddySupervision.initiate({
            ...args,
            endpoint_identity: buddyOperatorIdentity(store.getState()),
            operation: "create",
            request,
          }),
        )
        .unwrap(),
    ).rejects.toBeDefined();
    store.dispatch(buddySupervisionApi.util.resetApiState());
  });
  it("rejects native acknowledgement that changes the requested mandate or scope", async () => {
    const store = setUpStore({
      config: { host: "vscode", lspPort: 8001, themeProps: {} },
    });
    const request = buildSupervisionGrant(
      context,
      "m",
      "model",
      1,
      1,
      1,
      null,
      expiry(),
      {
        mode: "engineering",
        tools: ["apply_patch"],
        target_files: ["src/only.ts"],
      },
    );
    server.use(
      http.post("*/v1/buddy/supervision/mandates", () =>
        HttpResponse.json({
          revision: 1,
          change_seq: 1,
          value: {
            schema_version: 1,
            project,
            worker: args.worker,
            mandate: {
              ...request.mandate,
              model_constraints: ["other-model"],
            },
            worker_tools: ["create_textdoc"],
            worker_target_files: ["src/other.ts"],
            steering_notes: [],
          },
        }),
      ),
    );
    await expect(
      store
        .dispatch(
          buddySupervisionApi.endpoints.changeBuddySupervision.initiate({
            ...args,
            endpoint_identity: buddyOperatorIdentity(store.getState()),
            operation: "create",
            request,
          }),
        )
        .unwrap(),
    ).rejects.toBeDefined();
    store.dispatch(buddySupervisionApi.util.resetApiState());
  });
});

describe("explicit supervision grant", () => {
  it("preserves exact user model and finite budgets without broad authority", () => {
    const request = buildSupervisionGrant(
      context,
      "m",
      "provider/model",
      10,
      20000,
      10,
      25,
      expiry(),
    );
    expect(request.max_workers).toBe(1);
    expect(request.routine_choices).toEqual([]);
    expect(request.mandate.owner).toEqual(context.authenticated_owner);
    expect(request.mandate.project).toEqual(project);
    expect(request.mandate.model_constraints).toEqual(["provider/model"]);
    expect(request.mandate.allowed_operations).toEqual([
      "create_goal",
      "spawn_worker",
    ]);
    expect(request.mandate.allowed_authority_classes).toEqual(["read_only"]);
    expect(request.mandate.budget_limit).toEqual({
      max_turns: 10,
      max_tokens: 20000,
      max_duration_seconds: 600,
      max_cost_micros: 250000,
    });
  });
  it.each([0, -1, NaN, Infinity, 1.5, 101])(
    "rejects invalid turns %s",
    (turns) => {
      expect(() =>
        buildSupervisionGrant(
          context,
          "m",
          "model",
          turns,
          1,
          1,
          null,
          new Date(Date.now() + 60000).toISOString(),
        ),
      ).toThrow();
    },
  );
  it("rejects missing authority, unbounded budgets, fractional costs and invalid expiry", () => {
    const expiry = new Date(Date.now() + 60000).toISOString();
    for (const [tokens, minutes, cost] of [
      [0, 1, null],
      [1000001, 1, null],
      [1, 0, null],
      [1, 1441, null],
      [1, 1, 0.5],
      [1, 1, Number.MAX_SAFE_INTEGER],
    ] as const) {
      expect(() =>
        buildSupervisionGrant(
          context,
          "m",
          "model",
          1,
          tokens,
          minutes,
          cost,
          expiry,
        ),
      ).toThrow();
    }
    for (const expires of [
      "",
      "invalid",
      new Date(0).toISOString(),
      new Date(Date.now() + 90000000).toISOString(),
    ])
      expect(() =>
        buildSupervisionGrant(context, "m", "model", 1, 1, 1, null, expires),
      ).toThrow();
    expect(() =>
      buildSupervisionGrant(
        { ...context, authenticated_owner: null },
        "m",
        "model",
        1,
        1,
        1,
        null,
        expiry,
      ),
    ).toThrow();
    expect(() =>
      buildSupervisionGrant(
        { ...context, features: { ...context.features, supervision: false } },
        "m",
        "model",
        1,
        1,
        1,
        null,
        expiry,
      ),
    ).toThrow();
    expect(() =>
      buildSupervisionGrant(context, "m", " ", 1, 1, 1, null, expiry),
    ).toThrow();
  });
  it("builds bounded explicit criteria with byte rather than character limits", () => {
    expect(
      buildGoalIntake("Objective", " First proof\n\nSecond proof "),
    ).toEqual({
      content: "Objective",
      criteria: [
        { id: "C1", text: "First proof", verify_hint: null },
        { id: "C2", text: "Second proof", verify_hint: null },
      ],
    });
    for (const [content, criteria] of [
      ["", "proof"],
      ["goal", ""],
      ["é".repeat(8001), "proof"],
      ["goal", "é".repeat(1001)],
      ["goal", Array<string>(13).fill("proof").join("\n")],
    ])
      expect(() => buildGoalIntake(content, criteria)).toThrow();
  });
  it.each([
    "create",
    "intake",
    "spawn",
    "reconcile",
    "revoke",
    "status",
  ] as const)("sends all four identity headers for %s", async (operation) => {
    const store = setUpStore({
      config: { host: "vscode", lspPort: 8001, themeProps: {} },
    });
    const bound = {
      ...args,
      endpoint_identity: buddyOperatorIdentity(store.getState()),
    };
    let headers: Headers | undefined;
    server.use(
      http.all("*/v1/buddy/supervision/mandates*", ({ request }) => {
        headers = request.headers;
        return HttpResponse.json({}, { status: 409 });
      }),
    );
    if (operation === "status") {
      const request = store.dispatch(
        buddySupervisionApi.endpoints.getBuddySupervisionStatus.initiate(bound),
      );
      await request;
      request.unsubscribe();
    } else {
      await store.dispatch(
        buddySupervisionApi.endpoints.changeBuddySupervision.initiate(
          operation === "create"
            ? { ...bound, operation, request: grant() }
            : operation === "intake"
              ? {
                  ...bound,
                  operation,
                  request: buildGoalIntake("Goal", "Proof"),
                }
              : { ...bound, operation },
        ),
      );
    }
    expect(headers?.get("X-Refact-Project-Id")).toBe("p");
    expect(headers?.get("X-Refact-Worker-Id")).toBe("w");
    expect(headers?.get("X-Refact-Engine-Instance")).toBe("e");
    expect(headers?.get("X-Refact-Worker-Epoch")).toBe("1");
    store.dispatch(buddySupervisionApi.util.resetApiState());
  });
});
