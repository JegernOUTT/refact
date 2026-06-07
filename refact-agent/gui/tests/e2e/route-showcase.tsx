import React from "react";
import { createRoot } from "react-dom/client";
import { Provider } from "react-redux";
import { Flex } from "@radix-ui/themes";

import { setUpStore, type RootState } from "../../src/app/store";
import { Theme } from "../../src/components/Theme";
import { AbortControllerProvider } from "../../src/contexts/AbortControllers";
import { Dashboard } from "../../src/features/Dashboard";
import { Chat } from "../../src/features/Chat";
import { setBackendStatus } from "../../src/features/Connection";
import { sidebarSectionSnapshotReceived } from "../../src/features/Sidebar/sidebarSlice";
import { tasksApi, type TaskMeta } from "../../src/services/refact/tasks";
import type { ChatHistoryItem } from "../../src/features/History/historySlice";
import type { ChatThreadRuntime } from "../../src/features/Chat/Thread";
import "../../src/lib/render/web.css";

const now = "2026-06-07T10:00:00Z";
const chatId = "showcase-chat";

const chatRuntime: ChatThreadRuntime = {
  thread: {
    id: chatId,
    title: "Responsive route showcase chat",
    createdAt: now,
    updatedAt: now,
    model: "gpt-4o",
    tool_use: "agent",
    messages: [
      {
        role: "user",
        content:
          "Check that narrow screens do not create page horizontal scroll.",
      },
      {
        role: "assistant",
        content:
          "This harness renders the real chat shell with representative content for the e2e responsiveness gate.",
      },
    ],
    new_chat_suggested: { wasSuggested: false },
  },
  streaming: false,
  waiting_for_response: false,
  prevent_send: false,
  error: null,
  queued_items: [],
  send_immediately: false,
  attached_images: [],
  attached_text_files: [],
  background_agents: {},
  confirmation: {
    pause: false,
    pause_reasons: [],
    status: { wasInteracted: false, confirmationStatus: true },
  },
  snapshot_received: true,
  task_widget_expanded: false,
  memory_enrichment_user_touched: false,
  manual_preview_items: [],
  manual_preview_ran: false,
};

const historyItem = (
  id: string,
  title: string,
  updatedAt: string,
): ChatHistoryItem => ({
  id,
  title,
  createdAt: updatedAt,
  updatedAt,
  model: "gpt-4o",
  messages: [],
  new_chat_suggested: { wasSuggested: false },
  message_count: 3,
  mode: "agent",
});

const tasks: TaskMeta[] = [
  {
    id: "task-planning",
    name: "Responsive design-system migration",
    status: "planning",
    created_at: now,
    updated_at: now,
    cards_total: 37,
    cards_done: 5,
    cards_failed: 0,
    agents_active: 1,
  },
  {
    id: "task-active",
    name: "Token layer audit",
    status: "active",
    created_at: now,
    updated_at: now,
    cards_total: 8,
    cards_done: 3,
    cards_failed: 1,
    agents_active: 2,
  },
];

const preloadedState: Partial<RootState> = {
  config: {
    host: "web",
    lspPort: 8001,
    lspUrl: "",
    dev: false,
    engineServed: false,
    apiKey: null,
    features: { statistics: true, vecdb: true, ast: true, images: true },
    themeProps: { appearance: "dark" },
    shiftEnterToSubmit: false,
  },
  chat: {
    current_thread_id: chatId,
    open_thread_ids: [chatId],
    threads: { [chatId]: chatRuntime },
    max_new_tokens: 4096,
    tool_use: "agent",
    system_prompt: {},
    sse_refresh_requested: null,
    stream_version: 0,
  },
  history: {
    chats: {
      alpha: historyItem("alpha", "Dashboard route smoke coverage", now),
      beta: historyItem(
        "beta",
        "Chat surface responsive check",
        "2026-06-06T10:00:00Z",
      ),
    },
    isLoading: false,
    loadError: null,
    pagination: { cursor: null, hasMore: false, totalCount: 2, generation: 1 },
  },
  pages: [{ name: "history" }],
};

const store = setUpStore(preloadedState);

store.dispatch(setBackendStatus({ status: "online" }));
for (const section of ["workspace", "chats", "tasks", "buddy"] as const) {
  store.dispatch(sidebarSectionSnapshotReceived({ section, status: "ready" }));
}
store.dispatch(tasksApi.util.upsertQueryData("listTasks", undefined, tasks));

const route =
  new URLSearchParams(window.location.search).get("route") ?? "dashboard";
const root = document.getElementById("refact-chat");

if (!root) {
  throw new Error("Missing #refact-chat route showcase root");
}

const Showcase = () => {
  const surface =
    route === "chat" ? (
      <Chat host="web" tabbed={false} backFromChat={() => undefined} />
    ) : (
      <Dashboard />
    );

  return (
    <Provider store={store}>
      <Theme>
        <AbortControllerProvider>
          <Flex
            align="stretch"
            direction="column"
            data-element="app-root"
            style={{
              width: "100%",
              maxWidth: "100%",
              minWidth: 0,
              height: "100vh",
              overflow: "hidden",
            }}
          >
            {surface}
          </Flex>
        </AbortControllerProvider>
      </Theme>
    </Provider>
  );
};

createRoot(root).render(<Showcase />);
