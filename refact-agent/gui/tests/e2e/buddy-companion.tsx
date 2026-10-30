import { createRoot } from "react-dom/client";
import { Provider } from "react-redux";
import { setUpStore } from "../../src/app/store";
import { Theme } from "../../src/components/Theme";
import { FloatingBuddy } from "../../src/features/Buddy/Companion/FloatingBuddy";
import { createChatWithId } from "../../src/features/Chat/Thread";
import { registerVisibleChatMount } from "../../src/features/Connection";
import { push } from "../../src/features/Pages/pagesSlice";
import type { AttentionSnapshot } from "../../src/services/refact/buddyCompanion";
import "../../src/lib/render/web.css";

const root = document.getElementById("refact-chat");
if (!root) throw new Error("Buddy fixture root missing");
const params = new URLSearchParams(location.search);
const snapshot: AttentionSnapshot = {
  schema_version: 1,
  project_root: "/fixture/project",
  engine_identity: "fixture-engine",
  coverage: "loaded_current_project_sessions_and_child_registry",
  partial: true,
  busy_chat_ids: ["busy-source"],
  rows: [
    {
      id: "request-b",
      kind: "tool_permission",
      chat_id: "source-b",
      source_version: "v1",
      reason: "This command requires your permission before execution.",
      effect: "execute_pending_tool",
      actionable: true,
      literal_command: "npm run test -- --run",
      display_command: "shell npm run test -- --run",
      cwd: null,
      tool_call_id: "tool-b",
      question: null,
      parent_chat_id: null,
      run_generation: null,
      expires_at: null,
      tool_name: "shell",
      tool_arguments: {
        command: "npm run test -- --run",
        workdir: "/fixture/project",
      },
    },
  ],
};
// The real connected component and service run against an explicitly isolated fixture transport.
window.fetch = async (input, init) => {
  const url = new URL(
    typeof input === "string"
      ? input
      : input instanceof URL
        ? input.href
        : input.url,
    location.origin,
  );
  if (url.pathname === "/v1/buddy/attention") return Response.json(snapshot);
  if (url.pathname === "/v1/buddy/approvals/source-b/response")
    return Response.json({ queued: true }, { status: 202 });
  if (url.pathname.includes("/outcomes/"))
    return Response.json({
      project_root: snapshot.project_root,
      engine_identity: snapshot.engine_identity,
      chat_id: "source-b",
      client_request_id: url.pathname.split("/").at(-1),
      request_identity: "request-b",
      source_version: "v1",
      payload_hash: "fixture",
      target_id: "tool-b",
      accepted: true,
      status: "queued",
      followup: "pending",
    });
  if (url.pathname.startsWith("/v1/buddy/companion/"))
    return Response.json(
      {
        error:
          "Fixture intentionally does not run a model. Your draft stays in place.",
      },
      { status: 503 },
    );
  void init;
  return Response.json(
    { error: "No live backend is reachable from this isolated fixture" },
    { status: 503 },
  );
};
const store = setUpStore({
  config: {
    host: "web",
    dev: true,
    lspUrl: location.origin,
    themeProps: {
      appearance: params.get("theme") === "light" ? "light" : "dark",
    },
  },
});
store.dispatch(
  createChatWithId({
    id: "source-a",
    title: "Investigate failing tests",
    activate: true,
    openTab: false,
  }),
);
store.dispatch(
  createChatWithId({
    id: "source-b",
    title: "Verify package changes",
    activate: false,
    openTab: false,
  }),
);
store.dispatch(registerVisibleChatMount({ chatId: "source-a" }));
store.dispatch(push({ name: "chat" }));
createRoot(root).render(
  <Provider store={store}>
    <Theme>
      <main>
        <h1>Connected Buddy companion fixture</h1>
        <p>
          Source A is visible. Attention is from source B. No live backend or
          model is used.
        </p>
        <label htmlFor="main-draft">Main source draft</label>
        <textarea
          id="main-draft"
          defaultValue="Keep my investigation notes unchanged"
        />
        <FloatingBuddy />
      </main>
    </Theme>
  </Provider>,
);
