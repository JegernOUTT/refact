import "../../src/lib/render/web.css";
import { createRoot } from "react-dom/client";
import { Provider } from "react-redux";
import { setUpStore } from "../../src/app/store";
import { Theme } from "../../src/components/Theme";
import { AbortControllerProvider } from "../../src/contexts/AbortControllers";
import { STUB_CAPS_RESPONSE } from "../../src/__fixtures__/caps";
import { BuddyCompanion } from "../../src/features/Buddy/Companion/BuddyCompanion";
import {
  applyChatEvent,
  createChatWithId,
} from "../../src/features/Chat/Thread";
import { registerVisibleChatMount } from "../../src/features/Connection";
import { push } from "../../src/features/Pages/pagesSlice";
import { ActivityRail } from "../../src/features/Workspace/ActivityRail/ActivityRail";
import type {
  AttentionRow,
  AttentionSnapshot,
} from "../../src/services/refact/buddyCompanion";
import styles from "./buddy-companion.module.css";

const root = document.getElementById("refact-chat");
if (!root) throw new Error("Buddy fixture root missing");
const params = new URLSearchParams(location.search);
const companionId = "buddy-fixture";
const model = "gpt-4o";

function row(index: number): AttentionRow {
  return {
    id: `request-${index}`,
    kind: index % 2 === 0 ? "question" : "tool_permission",
    chat_id: `source-${index}`,
    source_version: "v1",
    reason:
      index % 2 === 0
        ? "The agent asked which test scope to run before continuing with the package verification."
        : "This command requires your permission before execution in the project workspace.",
    effect: "execute_pending_tool",
    actionable: true,
    literal_command: null,
    display_command: null,
    cwd: null,
    tool_call_id: `tool-${index}`,
    question: null,
    parent_chat_id: index === 1 ? "source-parent" : null,
    run_generation: null,
    expires_at: null,
  };
}

const rowCount = Number(params.get("rows") ?? "4");
const snapshot: AttentionSnapshot = {
  schema_version: 1,
  project_root: "/fixture/project",
  engine_identity: "fixture-engine",
  coverage: "full",
  partial: false,
  busy_chat_ids: [],
  rows: Array.from({ length: rowCount }, (_, index) => row(index + 1)),
};

const posts: string[] = [];
const commands: { chatId: string; type: unknown }[] = [];
Object.assign(window, { fixturePosts: posts, fixtureCommands: commands });
const nativeFetch = window.fetch.bind(window);
window.fetch = async (input, init) => {
  const url = new URL(
    typeof input === "string"
      ? input
      : input instanceof URL
        ? input.href
        : input.url,
    location.origin,
  );
  const path = url.pathname;
  if (!path.startsWith("/v1/")) return nativeFetch(input, init);
  const method =
    init?.method ?? (input instanceof Request ? input.method : "GET");
  if (method === "POST") posts.push(path);
  const commandPath = /^\/v1\/chats\/([^/]+)\/commands$/.exec(path);
  if (method === "POST" && commandPath) {
    const body: unknown =
      typeof init?.body === "string" ? JSON.parse(init.body) : null;
    commands.push({
      chatId: decodeURIComponent(commandPath[1] ?? ""),
      type:
        body && typeof body === "object" && "type" in body ? body.type : null,
    });
    return Response.json({ accepted: true }, { status: 202 });
  }
  if (path === "/v1/ping") return new Response("pong");
  if (path === "/v1/buddy/attention") return Response.json(snapshot);
  if (path.startsWith("/v1/buddy/companion/"))
    return Response.json({
      chat_id: companionId,
      source_chat_id: decodeURIComponent(path.split("/").at(-1) ?? ""),
      title: "Buddy about Investigate failing tests",
      kind: "companion",
      created_at: "2026-11-03T00:00:00Z",
    });
  if (path === "/v1/caps")
    return Response.json({ ...STUB_CAPS_RESPONSE, chat_default_model: model });
  if (path === "/v1/chat-modes")
    return Response.json({ modes: [], errors: [] });
  if (path === "/v1/at-command-completion")
    return Response.json({
      completions: [],
      replace: [0, 0],
      is_cmd_executable: false,
    });
  if (path === "/v1/at-command-preview") return Response.json({ messages: [] });
  if (path === "/v1/tools") return Response.json([]);
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
for (let index = 1; index <= rowCount; index++) {
  store.dispatch(
    createChatWithId({
      id: `source-${index}`,
      title: `Verify package changes ${index}`,
      activate: false,
      openTab: false,
    }),
  );
}
store.dispatch(
  applyChatEvent({
    chat_id: companionId,
    seq: "0",
    type: "snapshot",
    thread: {
      id: companionId,
      title: "Buddy about Investigate failing tests",
      model,
      mode: "no_tools",
      tool_use: "none",
      boost_reasoning: false,
      context_tokens_cap: null,
      include_project_info: false,
      checkpoints_enabled: false,
      is_title_generated: true,
      buddy_meta: {
        is_buddy_chat: true,
        buddy_chat_kind: "companion",
        workflow_id: null,
        source_chat_id: "source-a",
      },
    },
    runtime: {
      state: "idle",
      paused: false,
      error: null,
      queue_size: 0,
      pause_reasons: [],
      queued_items: [],
    },
    messages: [
      { role: "user", content: "Why are the tests failing?", message_id: "u1" },
      {
        role: "assistant",
        content:
          "The source chat is waiting on a shell permission. Once you approve it there, the test run continues.",
        message_id: "a1",
      },
    ],
    background_agents: [],
  }),
);
if (params.get("source") !== "none")
  store.dispatch(registerVisibleChatMount({ chatId: "source-a" }));
store.dispatch(push({ name: "chat" }));

createRoot(root).render(
  <Provider store={store}>
    <Theme>
      <AbortControllerProvider>
        <div className={styles.shell}>
          <ActivityRail />
          <main className={styles.main}>
            <h1>Connected Buddy companion fixture</h1>
            <label htmlFor="main-draft">Main source draft</label>
            <textarea
              id="main-draft"
              className={styles.draft}
              defaultValue="Keep my investigation notes unchanged"
            />
          </main>
        </div>
        <BuddyCompanion />
      </AbortControllerProvider>
    </Theme>
  </Provider>,
);
