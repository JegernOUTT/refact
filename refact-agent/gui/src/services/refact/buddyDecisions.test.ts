import { configureStore } from "@reduxjs/toolkit";
import { afterEach, describe, expect, test, vi } from "vitest";

import { buddyApi } from "./buddy";
import {
  buddyDecisionsApi,
  parseNativeDecisionSnapshot,
} from "./buddyDecisions";
import { buddyOperatorIdentity } from "./buddyOperator";
import type { EngineApiConfig } from "./apiUrl";

type FetchLike = (
  input: RequestInfo | URL,
  init?: RequestInit,
) => Promise<Response>;

type Config = EngineApiConfig & { apiKey: string | null };
type Project = { name: string; workspaceRoots: string[] };

function store(config: Config, project: Project) {
  return configureStore({
    reducer: {
      config: (state: Config = config) => state,
      current_project: (
        state: Project = project,
        action: { type: string; payload?: Project },
      ) =>
        action.type === "project/set" && action.payload
          ? action.payload
          : state,
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

const question = {
  tool_call_id: "ask-1",
  asked_at_ms: 1,
  status: "pending",
  questions: [{ id: "continue", type: "yes_no", text: "Continue?" }],
};
const row = { project_root: "/repo", chat_id: "chat-a", question };
const snapshot = {
  schema_version: 1,
  project_root: "/repo",
  coverage: "loaded_native_sessions",
  truncated: false,
  questions: [row],
};

afterEach(() => vi.unstubAllGlobals());

describe("Buddy native decisions API", () => {
  test("rejects reads whose requested endpoint identity is not the current one", async () => {
    const fetch = vi.fn<FetchLike>();
    vi.stubGlobal("fetch", fetch);
    const testStore = store(
      { host: "ide", lspPort: 8123, apiKey: null },
      { name: "project-a", workspaceRoots: ["/repo"] },
    );

    const staleList = await testStore.dispatch(
      buddyDecisionsApi.endpoints.getBuddyNativeDecisions.initiate({
        endpoint_identity: "old-project",
      }),
    );
    const staleQuestion = await testStore.dispatch(
      buddyDecisionsApi.endpoints.getBuddyNativeQuestion.initiate({
        project_root: "/repo",
        chat_id: "chat-a",
        tool_call_id: "ask-1",
        endpoint_identity: "old-project",
      }),
    );
    const staleAnswer = await testStore.dispatch(
      buddyDecisionsApi.endpoints.answerBuddyNativeQuestion.initiate({
        chat_id: "chat-a",
        tool_call_id: "ask-1",
        project_root: "/repo",
        asked_at_ms: 1,
        client_request_id: "request-1",
        endpoint_identity: "old-project",
        answers: [{ id: "continue", answer: true }],
      }),
    );

    expect(staleList.error).toMatchObject({ status: "CUSTOM_ERROR" });
    expect(staleQuestion.error).toMatchObject({ status: "CUSTOM_ERROR" });
    expect(staleAnswer.error).toMatchObject({ status: "CUSTOM_ERROR" });
    expect(fetch).not.toHaveBeenCalled();
    testStore.dispatch(buddyApi.util.resetApiState());
  });

  test("rejects in-flight reads and mutations when the project changes mid-request", async () => {
    const fetch = vi.fn<FetchLike>();
    vi.stubGlobal("fetch", fetch);
    const testStore = store(
      { host: "ide", lspPort: 8123, apiKey: null },
      { name: "project-a", workspaceRoots: ["/repo"] },
    );
    const identity = buddyOperatorIdentity(testStore.getState());
    const switchProject = () =>
      testStore.dispatch({
        type: "project/set",
        payload: { name: "project-b", workspaceRoots: ["/other"] },
      });

    fetch.mockImplementationOnce(() => {
      switchProject();
      return Promise.resolve(response(snapshot));
    });
    const list = await testStore.dispatch(
      buddyDecisionsApi.endpoints.getBuddyNativeDecisions.initiate({
        endpoint_identity: identity,
      }),
    );
    expect(list.error).toMatchObject({ status: "CUSTOM_ERROR" });
    expect(list.data).toBeUndefined();

    testStore.dispatch({
      type: "project/set",
      payload: { name: "project-a", workspaceRoots: ["/repo"] },
    });
    fetch.mockImplementationOnce(() => {
      switchProject();
      return Promise.resolve(response({ status: "queued" }));
    });
    const answer = await testStore.dispatch(
      buddyDecisionsApi.endpoints.answerBuddyNativeQuestion.initiate({
        chat_id: "chat-a",
        tool_call_id: "ask-1",
        project_root: "/repo",
        asked_at_ms: 1,
        client_request_id: "request-2",
        endpoint_identity: identity,
        answers: [{ id: "continue", answer: true }],
      }),
    );
    expect(answer.error).toMatchObject({ status: "CUSTOM_ERROR" });
    expect(fetch).toHaveBeenCalledTimes(2);
    testStore.dispatch(buddyApi.util.resetApiState());
  });

  test("rejects native question responses that do not match the requested identity", async () => {
    const fetch = vi.fn<FetchLike>();
    vi.stubGlobal("fetch", fetch);
    const testStore = store(
      { host: "ide", lspPort: 8123, apiKey: null },
      { name: "project-a", workspaceRoots: ["/repo"] },
    );
    const identity = buddyOperatorIdentity(testStore.getState());
    const args = {
      project_root: "/repo",
      chat_id: "chat-a",
      tool_call_id: "ask-1",
      endpoint_identity: identity,
    };

    for (const changed of [
      { ...row, project_root: "/other" },
      { ...row, chat_id: "chat-b" },
      { ...row, question: { ...question, tool_call_id: "ask-2" } },
    ]) {
      fetch.mockResolvedValueOnce(response(changed));
      const result = await testStore.dispatch(
        buddyDecisionsApi.endpoints.getBuddyNativeQuestion.initiate(args, {
          forceRefetch: true,
        }),
      );
      expect(result.error).toMatchObject({ status: "CUSTOM_ERROR" });
      expect(result.data).toBeUndefined();
    }

    fetch.mockResolvedValueOnce(response(row));
    const ok = await testStore.dispatch(
      buddyDecisionsApi.endpoints.getBuddyNativeQuestion.initiate(args, {
        forceRefetch: true,
      }),
    );
    expect(ok.data?.chat_id).toBe("chat-a");
    expect(ok.data?.question.tool_call_id).toBe("ask-1");
    testStore.dispatch(buddyApi.util.resetApiState());
  });

  test("keeps per-endpoint cache entries separate for different endpoint identities", async () => {
    const fetch = vi.fn<FetchLike>();
    vi.stubGlobal("fetch", fetch);
    const testStore = store(
      { host: "ide", lspPort: 8123, apiKey: null },
      { name: "project-a", workspaceRoots: ["/repo"] },
    );
    const identityA = buddyOperatorIdentity(testStore.getState());
    fetch.mockResolvedValueOnce(response(snapshot));
    const first = await testStore.dispatch(
      buddyDecisionsApi.endpoints.getBuddyNativeDecisions.initiate({
        endpoint_identity: identityA,
      }),
    );
    expect(first.data?.questions).toHaveLength(1);

    testStore.dispatch({
      type: "project/set",
      payload: { name: "project-b", workspaceRoots: ["/other"] },
    });
    const identityB = buddyOperatorIdentity(testStore.getState());
    expect(identityB).not.toBe(identityA);
    fetch.mockResolvedValueOnce(
      response({
        ...snapshot,
        project_root: "/other",
        questions: [{ ...row, project_root: "/other" }],
      }),
    );
    const second = await testStore.dispatch(
      buddyDecisionsApi.endpoints.getBuddyNativeDecisions.initiate({
        endpoint_identity: identityB,
      }),
    );
    expect(second.data?.project_root).toBe("/other");

    const cachedA = buddyDecisionsApi.endpoints.getBuddyNativeDecisions.select({
      endpoint_identity: identityA,
    })(testStore.getState() as never);
    expect(cachedA.data?.project_root).toBe("/repo");
    testStore.dispatch(buddyApi.util.resetApiState());
  });

  test("rejects snapshots with mismatched project roots or duplicate question identities", () => {
    expect(() =>
      parseNativeDecisionSnapshot({
        ...snapshot,
        questions: [{ ...row, project_root: "/elsewhere" }],
      }),
    ).toThrow();
    expect(() =>
      parseNativeDecisionSnapshot({ ...snapshot, questions: [row, row] }),
    ).toThrow();
    expect(
      parseNativeDecisionSnapshot({
        ...snapshot,
        questions: [
          row,
          { ...row, question: { ...question, tool_call_id: "ask-2" } },
        ],
      }).questions,
    ).toHaveLength(2);
  });

  test("preserves transport failures instead of reporting native questions as empty", async () => {
    const fetch = vi.fn<FetchLike>();
    vi.stubGlobal("fetch", fetch);
    const testStore = store(
      { host: "ide", lspPort: 8123, apiKey: null },
      { name: "project-a", workspaceRoots: ["/repo"] },
    );
    const identity = buddyOperatorIdentity(testStore.getState());
    fetch.mockResolvedValueOnce(response({ error: "unavailable" }, 503));
    const result = await testStore.dispatch(
      buddyDecisionsApi.endpoints.getBuddyNativeDecisions.initiate({
        endpoint_identity: identity,
      }),
    );
    expect(result.error).toMatchObject({ status: 503 });
    expect(result.data).toBeUndefined();
    testStore.dispatch(buddyApi.util.resetApiState());
  });
});
