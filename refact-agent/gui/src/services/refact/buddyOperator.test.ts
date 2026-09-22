import { configureStore } from "@reduxjs/toolkit";
import { afterEach, describe, expect, test, vi } from "vitest";

import { buddyApi } from "./buddy";
import {
  buddyOperatorApi,
  buddyOperatorIdentity,
  parseOperatorStatus,
} from "./buddyOperator";
import type { EngineApiConfig } from "./apiUrl";

type FetchLike = (
  input: RequestInfo | URL,
  init?: RequestInit,
) => Promise<Response>;

type Config = EngineApiConfig & { apiKey: string | null };

function store(config: Config) {
  return configureStore({
    reducer: {
      config: (state: Config = config) => state,
      [buddyApi.reducerPath]: buddyApi.reducer,
    },
    middleware: (getDefaultMiddleware) =>
      getDefaultMiddleware().concat(buddyApi.middleware),
  });
}

function response(data: unknown, status = 200): Response {
  return new Response(JSON.stringify(data), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

function envelope(data: unknown, next_cursor: number | null = null) {
  const record = (value: unknown) => ({ value, revision: 1, change_seq: 1 });
  return {
    schema_version: 1,
    project: { project_id: "project-1", canonical_root: "/repo" },
    data: Array.isArray(data) ? data.map(record) : record(data),
    next_cursor,
  };
}

function work(id = "work-1") {
  return {
    schema_version: 1,
    id,
    project: { project_id: "project-1", canonical_root: "/repo" },
    subject: { resource_kind: "task", resource_id: "task-1", version: "1" },
    kind: "task",
    title: "Work",
    state: "working",
    native_sources: [],
    current_artifact_id: null,
    current_artifact_revision: null,
    last_material_change_at: "2026-01-01T00:00:00Z",
  };
}

function artifact() {
  return {
    schema_version: 1,
    artifact_id: "artifact-1",
    revision: 1,
    project: { project_id: "project-1", canonical_root: "/repo" },
    source_run_ids: [],
    title: "Artifact",
    conclusion: "done",
    delta: "",
    body: "body",
    uncertainty: null,
    proposed_action_ids: [],
    evidence: [],
    provenance: {
      source_run_ids: [],
      evidence_refs: [],
      contains_untrusted_source: false,
    },
  };
}

function execution() {
  return {
    schema_version: 1,
    execution_id: "execution-1",
    project: { project_id: "project-1", canonical_root: "/repo" },
    worker: { worker_id: "worker", engine_instance_id: "engine", epoch: 1 },
    idempotency_key: "key",
    action_id: "action",
    spec_hash: "hash",
    requester: {
      source_kind: "workflow",
      source_id: "run",
      source_version: "1",
    },
    decision: null,
    status: "completed",
    started_at: null,
    ended_at: null,
    effects: [],
    verification_evidence: [],
    recovery: null,
  };
}

afterEach(() => vi.unstubAllGlobals());

describe("Buddy operator read API", () => {
  test("snooze contracts require matching project, report and explicit endpoint identity", async () => {
    const fetch = vi.fn<FetchLike>();
    vi.stubGlobal("fetch", fetch);
    const testStore = store({ host: "ide", lspPort: 8123, apiKey: null });
    const args = {
      id: "work-1",
      project_id: "project-1",
      canonical_root: "/repo",
      artifact_id: "artifact-1",
      artifact_revision: 1,
      minutes: 60,
    };
    const stale = await testStore.dispatch(
      buddyOperatorApi.endpoints.snoozeBuddyWork.initiate({
        ...args,
        endpoint_identity: "old",
      }),
    );
    expect(stale.error).toMatchObject({ status: "CUSTOM_ERROR" });
    expect(fetch).not.toHaveBeenCalled();
    const identity = buddyOperatorIdentity(testStore.getState());
    const snooze = {
      schema_version: 1,
      project: { project_id: "project-1", canonical_root: "/repo" },
      work_id: "work-1",
      artifact_id: "artifact-1",
      artifact_revision: 1,
      until: "2099-01-01T00:00:00Z",
    };
    for (const changed of [
      snooze,
      { ...snooze, artifact_revision: 2 },
      { ...snooze, until: "not-a-date" },
    ]) {
      fetch.mockResolvedValueOnce(
        response({ schema_version: 1, project: snooze.project, data: changed }),
      );
      const result = await testStore.dispatch(
        buddyOperatorApi.endpoints.snoozeBuddyWork.initiate({
          ...args,
          endpoint_identity: identity,
        }),
      );
      if (changed === snooze) expect(result.data?.data).toEqual(snooze);
      else expect(result.error).toMatchObject({ status: "PARSING_ERROR" });
    }
    testStore.dispatch(buddyApi.util.resetApiState());
  });
  test("rejects a stale endpoint before sending an execution and rejects mismatched receipts", async () => {
    const fetch = vi.fn<FetchLike>();
    vi.stubGlobal("fetch", fetch);
    const testStore = store({ host: "ide", lspPort: 8123, apiKey: "token" });
    const args = {
      id: "work-1",
      action_id: "action",
      project_id: "project-1",
      canonical_root: "/repo",
      work_revision: 1,
      artifact_id: "artifact-1",
      artifact_revision: 1,
      spec_hash: "hash",
      idempotency_key: "key",
    };
    const stale = await testStore.dispatch(
      buddyOperatorApi.endpoints.executeBuddyDeskAction.initiate({
        ...args,
        endpoint_identity: "old-engine",
      }),
    );
    expect(stale.error).toMatchObject({ status: "CUSTOM_ERROR" });
    expect(fetch).not.toHaveBeenCalled();
    for (const changed of [
      { ...execution(), spec_hash: "other" },
      { ...execution(), action_id: "other" },
      { ...execution(), idempotency_key: "other" },
    ]) {
      fetch.mockResolvedValueOnce(response(envelope(changed)));
      const result = await testStore.dispatch(
        buddyOperatorApi.endpoints.executeBuddyDeskAction.initiate(args),
      );
      expect(result.error).toMatchObject({ status: "PARSING_ERROR" });
    }
    fetch.mockResolvedValueOnce(response(envelope(execution())));
    const wrongRoot = await testStore.dispatch(
      buddyOperatorApi.endpoints.executeBuddyDeskAction.initiate({
        ...args,
        canonical_root: "/other",
      }),
    );
    expect(wrongRoot.error).toMatchObject({ status: "PARSING_ERROR" });
  });

  test("parses list, artifact, and execution response envelopes", async () => {
    const fetch = vi.fn<FetchLike>();
    vi.stubGlobal("fetch", fetch);
    fetch
      .mockResolvedValueOnce(response(envelope([work()], 9)))
      .mockResolvedValueOnce(response(envelope(artifact())))
      .mockResolvedValueOnce(response(envelope(execution())));
    const testStore = store({ host: "ide", lspPort: 8123, apiKey: "token" });

    const list = await testStore.dispatch(
      buddyOperatorApi.endpoints.getBuddyOperatorWork.initiate({ limit: 10 }),
    );
    const artifactResult = await testStore.dispatch(
      buddyOperatorApi.endpoints.getBuddyOperatorArtifact.initiate({
        id: "artifact-1",
      }),
    );
    const executionResult = await testStore.dispatch(
      buddyOperatorApi.endpoints.getBuddyOperatorExecution.initiate({
        id: "execution-1",
      }),
    );

    expect(list.data?.data[0].value.id).toBe("work-1");
    expect(list.data?.next_cursor).toBe(9);
    expect(artifactResult.data?.data.value.artifact_id).toBe("artifact-1");
    expect(executionResult.data?.data.value.execution_id).toBe("execution-1");
  });

  test("encodes unsafe ids and includes pagination query parameters", async () => {
    const fetch = vi.fn<FetchLike>();
    vi.stubGlobal("fetch", fetch);
    fetch.mockResolvedValueOnce(response(envelope(work("unsafe/id"))));
    const testStore = store({ host: "ide", lspPort: 8123, apiKey: null });

    await testStore.dispatch(
      buddyOperatorApi.endpoints.getBuddyOperatorWorkItem.initiate({
        id: "unsafe/id?x=1",
        project_id: "project-1",
      }),
    );
    const [input] = fetch.mock.calls[0];
    const request = input instanceof Request ? input : new Request(input);
    expect(new URL(request.url).pathname).toBe(
      "/v1/buddy/work/unsafe%2Fid%3Fx%3D1",
    );
    expect(new URL(request.url).searchParams.get("project_id")).toBe(
      "project-1",
    );

    fetch.mockResolvedValueOnce(response(envelope([work()], 4)));
    await testStore.dispatch(
      buddyOperatorApi.endpoints.getBuddyOperatorWork.initiate({
        cursor: 3,
        limit: 2,
      }),
    );
    const [listInput] = fetch.mock.calls[1];
    const listUrl = new URL(
      listInput instanceof Request ? listInput.url : String(listInput),
    );
    expect(listUrl.searchParams.get("cursor")).toBe("3");
    expect(listUrl.searchParams.get("limit")).toBe("2");
  });

  test("rejects malformed and unsupported envelopes rather than accepting unsafe data", async () => {
    const fetch = vi.fn<FetchLike>();
    vi.stubGlobal("fetch", fetch);
    fetch.mockResolvedValueOnce(
      response({ schema_version: 2, project: {}, data: [] }),
    );
    const testStore = store({ host: "ide", lspPort: 8123, apiKey: null });

    const result = await testStore.dispatch(
      buddyOperatorApi.endpoints.getBuddyOperatorWork.initiate(undefined),
    );
    expect(result.error).toMatchObject({ status: "PARSING_ERROR" });
  });

  test("preserves unavailable transport responses", async () => {
    const fetch = vi.fn<FetchLike>();
    vi.stubGlobal("fetch", fetch);
    fetch.mockResolvedValueOnce(
      response({ error: "operator unavailable" }, 503),
    );
    const testStore = store({ host: "ide", lspPort: 8123, apiKey: null });

    const result = await testStore.dispatch(
      buddyOperatorApi.endpoints.getBuddyOperatorExecution.initiate({
        id: "execution-1",
      }),
    );
    expect(result.error).toMatchObject({ status: 503 });
  });
});

describe("operator status runtime contract", () => {
  const enabled = {
    schema_version: 1,
    mode: "enabled",
    features: {
      storage: true,
      actions: false,
      supervision: false,
      improvement: false,
      editorial: false,
    },
    project: { project_id: "p", canonical_root: "/p" },
    worker: { worker_id: "w", engine_instance_id: "e", epoch: 0 },
  };
  test("accepts the additive unavailable reason and old status shape", () => {
    expect(parseOperatorStatus(enabled)).toEqual(enabled);
    for (const mode of ["disabled", "unavailable"]) {
      const value = {
        ...enabled,
        mode,
        features: { ...enabled.features, storage: false },
        project: null,
        worker: null,
      };
      expect(parseOperatorStatus(value)).toEqual(value);
    }
    const unavailable = {
      ...enabled,
      mode: "unavailable",
      features: { ...enabled.features, storage: false },
      project: null,
      worker: null,
      unavailable_reason: {
        kind: "already_owned",
        detail: "pid 2810136 (refact-lsp) owns the store",
        retry_in_seconds: 900,
      },
    };
    expect(parseOperatorStatus(unavailable)).toEqual(unavailable);
  });
  test.each([
    null,
    [],
    {},
    { ...enabled, extra: true },
    { ...enabled, schema_version: 2 },
    { ...enabled, mode: "unknown" },
    { ...enabled, mode: "disabled" },
    { ...enabled, features: null },
    { ...enabled, features: [] },
    { ...enabled, features: { ...enabled.features, actions: "false" } },
    { ...enabled, features: { ...enabled.features, extra: true } },
    { ...enabled, features: { ...enabled.features, storage: false } },
    { ...enabled, project: null },
    { ...enabled, project: undefined },
    { ...enabled, project: { ...enabled.project, extra: true } },
    { ...enabled, project: { ...enabled.project, project_id: " " } },
    { ...enabled, worker: null },
    { ...enabled, worker: undefined },
    { ...enabled, worker: [] },
    { ...enabled, worker: { ...enabled.worker, epoch: "1" } },
    { ...enabled, worker: { ...enabled.worker, epoch: -1 } },
    { ...enabled, worker: { ...enabled.worker, epoch: 0.5 } },
    { ...enabled, worker: { ...enabled.worker, extra: true } },
    { ...enabled, worker: { ...enabled.worker, worker_id: " " } },
    {
      ...enabled,
      unavailable_reason: { kind: "already_owned", detail: "holder" },
    },
    {
      ...enabled,
      mode: "unavailable",
      features: { ...enabled.features, storage: false },
      project: null,
      worker: null,
      unavailable_reason: { kind: "", detail: "holder" },
    },
    {
      ...enabled,
      mode: "unavailable",
      features: { ...enabled.features, storage: false },
      project: null,
      worker: null,
      unavailable_reason: {
        kind: "already_owned",
        detail: "",
        retry_in_seconds: -1,
      },
    },
  ])("rejects malformed status %#", (value) =>
    expect(() => parseOperatorStatus(value)).toThrow(),
  );
});
