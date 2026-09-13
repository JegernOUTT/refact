import { configureStore } from "@reduxjs/toolkit";
import { afterAll, afterEach, beforeAll, describe, expect, test } from "vitest";
import { http, HttpResponse } from "msw";
import { setupServer } from "msw/node";
import { buddyApi } from "./buddy";
import { buddyOperatorIdentity } from "./buddyOperator";
import { buddyResourceActionsApi } from "./buddyResourceActions";
import type { ActionSpec } from "../../features/Buddy/operatorTypes";

const config = { lspPort: 8001, apiKey: "fixture" };
const spec: ActionSpec = {
  schema_version: 1,
  action_id: "apply",
  adapter: "settings",
  kind: "settings_trajectory_patch",
  project: { project_id: "p", canonical_root: "/repo" },
  resource: {
    resource_kind: "settings",
    resource_id: "trajectory",
    version: "v1",
  },
  expected_version: "v1",
  parameter_ref: "draft",
  availability: "available",
  authority_class: "mutating",
  confirmation: "user_confirmation_required",
  capabilities: {
    can_preview: true,
    can_execute: true,
    can_verify: true,
    can_recover: true,
    can_cancel: false,
  },
};
const args = {
  spec,
  endpoint_identity: buddyOperatorIdentity({ config }),
  spec_hash: "hash",
  grant_id: "grant",
  execution_id: "exec",
  idempotency_key: "key",
};
const envelope = (data: unknown) => ({
  schema_version: 1,
  project: spec.project,
  next_cursor: null,
  data,
});
const server = setupServer();
beforeAll(() => server.listen({ onUnhandledRequest: "error" }));
afterEach(() => server.resetHandlers());
afterAll(() => server.close());
function store() {
  return configureStore({
    reducer: { config: () => config, [buddyApi.reducerPath]: buddyApi.reducer },
    middleware: (gdm) => gdm().concat(buddyApi.middleware),
  });
}
function receipt(status: string) {
  const execution = {
    schema_version: 1,
    execution_id: "exec",
    project: spec.project,
    worker: { worker_id: "w", engine_instance_id: "e", epoch: 1 },
    idempotency_key: "key",
    action_id: spec.action_id,
    spec_hash: "hash",
    requester: { source_kind: "external", source_id: "u", source_version: "1" },
    decision: null,
    status,
    started_at: null,
    ended_at: null,
    effects: [],
    verification_evidence: [],
    recovery: null,
  };
  return { execution, effects: [], verification_evidence: [], recovery: null };
}
describe("real injected resource client", () => {
  test.each(["partial", "outcome_unknown"])(
    "preserves %s and Duplicate without replay",
    async (state) => {
      let calls = 0;
      server.use(
        http.post("*/v1/buddy/actions/execute", () => {
          calls++;
          return HttpResponse.json(envelope({ Duplicate: receipt(state) }));
        }),
      );
      const result = await store()
        .dispatch(
          buddyResourceActionsApi.endpoints.executeBuddyResource.initiate(args),
        )
        .unwrap();
      expect(result.disposition).toBe("Duplicate");
      expect(result.execution.status).toBe(state);
      expect(calls).toBe(1);
    },
  );
  test.each(["stale", "unknown", "project"])(
    "rejects %s preview contracts",
    async (kind) => {
      const data = {
        resource_id: "trajectory",
        expected_version: kind === "stale" ? "v0" : "v1",
        proposed_version: "v2",
        reload_needed: false,
        diff: "diff",
        ...(kind === "unknown" ? { unsafe: true } : {}),
      };
      server.use(
        http.post("*/v1/buddy/actions/preview", () =>
          HttpResponse.json({
            ...envelope(data),
            ...(kind === "project"
              ? { project: { ...spec.project, project_id: "other" } }
              : {}),
          }),
        ),
      );
      const result = await store().dispatch(
        buddyResourceActionsApi.endpoints.previewBuddyResource.initiate(args),
      );
      expect(result.error).toBeDefined();
    },
  );
  test("permission denial is not retried", async () => {
    let calls = 0;
    server.use(
      http.post("*/v1/buddy/actions/grant", () => {
        calls++;
        return new HttpResponse(null, { status: 403 });
      }),
    );
    const result = await store().dispatch(
      buddyResourceActionsApi.endpoints.grantBuddyResource.initiate({
        ...args,
        native_rights: null,
        recovery_execution_id: null,
      }),
    );
    expect(result.error).toBeDefined();
    expect(calls).toBe(1);
  });
  test("endpoint change blocks network entirely", async () => {
    const result = await store().dispatch(
      buddyResourceActionsApi.endpoints.previewBuddyResource.initiate({
        ...args,
        endpoint_identity: "old",
      }),
    );
    expect(result.error).toBeDefined();
  });
});
