import { configureStore } from "@reduxjs/toolkit";
import { afterEach, describe, expect, it, vi } from "vitest";
import { buddyApi } from "./buddy";
import { buddyApprovalsApi, parseNativeApprovals } from "./buddyApprovals";
import { buddyOperatorIdentity } from "./buddyOperator";

const session = {
  chat_id: "chat",
  source_version: "a".repeat(64),
  tools: [],
  browser_request_id: null,
  warnings: [],
};
const snapshot = {
  schema_version: 1,
  project_root: "/repo",
  engine_identity: "engine",
  coverage: "loaded_native_sessions",
  truncated: false,
  warnings: [],
  sessions: [session],
};
function store() {
  return configureStore({
    reducer: {
      config: () => ({ host: "ide" as const, lspPort: 8123, apiKey: null }),
      current_project: (
        state: { name: string; workspaceRoots: string[] } = {
          name: "a",
          workspaceRoots: ["/repo"],
        },
        action: { type: string },
      ) =>
        action.type === "switch"
          ? { name: "b", workspaceRoots: ["/other"] }
          : state,
      [buddyApi.reducerPath]: buddyApi.reducer,
    },
    middleware: (getDefaultMiddleware) =>
      getDefaultMiddleware().concat(buddyApi.middleware),
  });
}
afterEach(() => vi.unstubAllGlobals());

describe("native approval transport", () => {
  it("rejects unsupported schemas, source versions, coverage, duplicate IDs and unknown permission types", () => {
    expect(parseNativeApprovals(snapshot).sessions).toHaveLength(1);
    for (const change of [
      { schema_version: 2 },
      { coverage: "all_sessions" },
      { project_root: "" },
      { sessions: [session, session] },
      { sessions: [{ ...session, source_version: "stale" }] },
      {
        sessions: [
          {
            ...session,
            tools: [
              {
                type: "ide_callback",
                tool_call_id: "tool",
                tool_name: "ide",
                command: "",
                rule: "",
              },
            ],
          },
        ],
      },
      {
        sessions: Array.from({ length: 201 }, (_, i) => ({
          ...session,
          chat_id: String(i),
        })),
      },
    ])
      expect(() => parseNativeApprovals({ ...snapshot, ...change })).toThrow();
  });

  it("does not send stale endpoint reads or mutations", async () => {
    const fetch = vi.fn();
    vi.stubGlobal("fetch", fetch);
    const testStore = store();
    const query = await testStore.dispatch(
      buddyApprovalsApi.endpoints.getBuddyNativeApprovals.initiate({
        endpoint_identity: "stale",
      }),
    );
    const mutation = await testStore.dispatch(
      buddyApprovalsApi.endpoints.respondBuddyNativeApproval.initiate({
        endpoint_identity: "stale",
        chat_id: "chat",
        project_root: "/repo",
        engine_identity: "engine",
        source_version: "a".repeat(64),
        client_request_id: "request",
        response: { kind: "tool", tool_call_id: "tool", accepted: false },
      }),
    );
    expect(query.error).toBeTruthy();
    expect(mutation.error).toBeTruthy();
    expect(fetch).not.toHaveBeenCalled();
    testStore.dispatch(buddyApi.util.resetApiState());
  });

  it("discards a snapshot received after the project changes", async () => {
    const testStore = store();
    const identity = buddyOperatorIdentity(testStore.getState());
    vi.stubGlobal(
      "fetch",
      vi.fn(() => {
        testStore.dispatch({ type: "switch" });
        return Promise.resolve(
          new Response(JSON.stringify(snapshot), {
            headers: { "Content-Type": "application/json" },
          }),
        );
      }),
    );
    const result = await testStore.dispatch(
      buddyApprovalsApi.endpoints.getBuddyNativeApprovals.initiate({
        endpoint_identity: identity,
      }),
    );
    expect(result.error).toBeTruthy();
    expect(result.data).toBeUndefined();
    testStore.dispatch(buddyApi.util.resetApiState());
  });
});
