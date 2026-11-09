import { afterAll, afterEach, beforeAll } from "vitest";
import { http, HttpResponse } from "msw";
import { setupServer } from "msw/node";
import type { Store } from "../app/store";
import {
  capsApi,
  promptsApi,
  toolsApi,
  commandsApi,
  pingApi,
  taskDocumentsApi,
  daemonApi,
  filesApi,
  performanceApi,
} from "../services/refact";

export * from "../__fixtures__/msw";

export const resetApi = (store: Store) => {
  store.dispatch(capsApi.util.resetApiState());
  store.dispatch(promptsApi.util.resetApiState());
  store.dispatch(toolsApi.util.resetApiState());
  store.dispatch(commandsApi.util.resetApiState());
  store.dispatch(pingApi.util.resetApiState());
  store.dispatch(taskDocumentsApi.util.resetApiState());
  store.dispatch(daemonApi.util.resetApiState());
  store.dispatch(filesApi.util.resetApiState());
  store.dispatch(performanceApi.util.resetApiState());
};
// Home always mounts the native safety surfaces, even with intelligence off.
// Defaults model a real disabled rollout; tests can override each exact route.
export const server = setupServer(
  http.get("*/v1/reviews/by-chat/:chatId", () => HttpResponse.json(null)),
  http.get("*/v1/buddy/operator/status", () =>
    HttpResponse.json({
      schema_version: 1,
      mode: "disabled",
      project: null,
      worker: null,
      features: {
        storage: false,
        actions: false,
        supervision: false,
        improvement: false,
        editorial: false,
      },
    }),
  ),
  http.get("*/v1/buddy/operator/legacy-drafts/:id/work", () =>
    HttpResponse.json({ detail: "No mapped proposal" }, { status: 404 }),
  ),
  http.get("*/v1/buddy/decisions/current", () =>
    HttpResponse.json({
      schema_version: 1,
      project_root: "/fixture",
      coverage: "loaded_native_sessions",
      truncated: false,
      questions: [],
    }),
  ),
  http.get("*/v1/buddy/native-work", () =>
    HttpResponse.json({
      schema_version: 1,
      project_root: "/fixture",
      coverage: "loaded_native_sessions",
      next_cursor: null,
      sessions: [],
    }),
  ),
  http.get("*/v1/buddy/supervision/context", () =>
    HttpResponse.json({
      schema_version: 1,
      project: null,
      worker: null,
      authenticated_owner: null,
      features: {
        storage: false,
        actions: false,
        supervision: false,
        improvement: false,
        editorial: false,
      },
    }),
  ),
  http.get("*/v1/chats/activity", () => HttpResponse.json({ chats: [] })),
  http.get("*/v1/apps", () => HttpResponse.json({ apps: [], diagnostics: [] })),
  http.get("*/v1/apps/subscribe", () =>
    HttpResponse.text("", {
      headers: { "Content-Type": "text/event-stream" },
    }),
  ),
  http.get("*/v1/buddy/approvals/current", () =>
    HttpResponse.json({
      schema_version: 1,
      project_root: "/fixture",
      engine_identity: "fixture-engine",
      coverage: "loaded_native_sessions",
      truncated: false,
      warnings: [],
      sessions: [],
    }),
  ),
);

beforeAll(() => {
  // Enable the mocking in tests.
  server.listen({ onUnhandledRequest: "error" });
});

afterEach(() => {
  // Reset any runtime handlers tests may use.
  server.resetHandlers();
});

afterAll(() => {
  // Clean up once the tests are done.
  server.close();
});
