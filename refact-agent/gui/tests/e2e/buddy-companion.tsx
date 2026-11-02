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
import styles from "./buddy-companion.module.css";

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
const questionMode = params.get("question") === "1";
const question = {
  tool_call_id: "ask-fixture",
  asked_at_ms: 42,
  status: "pending" as const,
  response_protocol_version: 2,
  questions: [{ id: "scope", type: "free_text" as const, text: "What scope?" }],
};
if (questionMode) {
  snapshot.rows = [
    {
      ...snapshot.rows[0],
      id: "question-fixture",
      kind: "question",
      effect: "answer_question",
      tool_call_id: question.tool_call_id,
      question,
      literal_command: null,
      display_command: null,
      tool_arguments: null,
    },
  ];
}
const posts: string[] = [];
Object.assign(window, { fixturePosts: posts });
window.fetch = async (input, init) => {
  const url = new URL(
    typeof input === "string"
      ? input
      : input instanceof URL
        ? input.href
        : input.url,
    location.origin,
  );
  if (
    (init?.method ?? (input instanceof Request ? input.method : "GET")) ===
    "POST"
  )
    posts.push(url.pathname);
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
  if (questionMode && url.pathname.startsWith("/v1/buddy/companion/"))
    return Response.json({
      chat_id: "buddy-fixture",
      source_chat_id: "source-a",
      title: "Buddy fixture",
      kind: "companion",
      created_at: "now",
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
store.dispatch(
  createChatWithId({
    id: "buddy-fixture",
    title: "Buddy fixture",
    activate: false,
    openTab: false,
  }),
);
const fixtureChat = structuredClone(store.getState().chat);
fixtureChat.threads["buddy-fixture"].thread.messages = [
  {
    role: "assistant",
    content: "Buddy shared response",
    tool_calls: [
      {
        id: question.tool_call_id,
        index: 0,
        function: { name: "ask_questions", arguments: "{}" },
      },
    ],
  },
];
fixtureChat.threads["buddy-fixture"].thread.pending_questions = [question];

store.dispatch(registerVisibleChatMount({ chatId: "source-a" }));
store.dispatch(push({ name: "chat" }));
const fixtureStore = setUpStore({ ...store.getState(), chat: fixtureChat });
createRoot(root).render(
  <Provider store={questionMode ? fixtureStore : store}>
    <Theme>
      <main className={styles.main}>
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
