import { http, HttpResponse } from "msw";
import { afterEach, describe, expect, it, vi } from "vitest";

import { setUpStore } from "../app/store";
import { InnerApp } from "../features/App";
import { restoreChat } from "../features/Chat/Thread";
import type { ChatHistoryItem } from "../features/History/historySlice";
import { setBackendStatus } from "../features/Connection";
import { render, screen, waitFor } from "../utils/test-utils";
import {
  setProjectStorageNamespace,
  setProjectStorageNamespaceFromProjectInfo,
} from "../utils/chatUiPersistence";
import {
  chatLinks,
  chatSessionAbort,
  chatSessionCommand,
  chatSessionSubscribe,
  emptyTasks,
  goodCaps,
  goodPing,
  goodPrompts,
  goodTools,
  goodUser,
  noCommandPreview,
  server,
  sidebarSubscribe,
} from "../utils/mockServer";

vi.mock("../features/Chat/Chat", async () => {
  const React = await vi.importActual<typeof import("react")>("react");
  const thread = await vi.importActual<typeof import("../features/Chat/Thread")>(
    "../features/Chat/Thread",
  );
  const selectorHook = await vi.importActual<
    typeof import("../hooks/useAppSelector")
  >("../hooks/useAppSelector");

  return {
    Chat: ({ chatId }: { chatId?: string }) => {
      const currentThreadId = selectorHook.useAppSelector(
        thread.selectCurrentThreadId,
      );
      const resolvedChatId = chatId ?? currentThreadId;
      const messages = selectorHook.useAppSelector((state) =>
        thread.selectMessagesById(state, resolvedChatId),
      );

      return React.createElement(
        "section",
        { "data-testid": "single-chat", "data-chat-id": resolvedChatId },
        messages.map((message, index) =>
          React.createElement(
            "p",
            {
              key:
                "message_id" in message && message.message_id
                  ? message.message_id
                  : index,
            },
            typeof message.content === "string" ? message.content : "",
          ),
        ),
      );
    },
  };
});

vi.mock("../features/ChatPanes/ChatSplitLayout", async () => {
  const React = await vi.importActual<typeof import("react")>("react");

  return {
    ChatSplitLayout: () =>
      React.createElement(
        "div",
        { "data-testid": "split-chat-layout" },
        "Split chat layout",
      ),
  };
});

const appHandlers = [
  goodPing,
  goodUser,
  goodCaps,
  goodTools,
  goodPrompts,
  chatLinks,
  chatSessionSubscribe,
  chatSessionCommand,
  chatSessionAbort,
  emptyTasks,
  noCommandPreview,
  sidebarSubscribe,
  http.get("*/v1/chat-modes", () =>
    HttpResponse.json({ modes: [], errors: [] }),
  ),
  http.get("*/v1/setup/status", () =>
    HttpResponse.json({
      configured: true,
      reasons: [],
      detail: {
        project_root: "/tmp/refact-test",
        has_agents_md: true,
        has_knowledge: true,
        has_trajectories: true,
      },
    }),
  ),
  http.get("*/v1/voice/status", () => HttpResponse.json({ available: false })),
  http.get("*/v1/chats/:chatId/skills-status", () =>
    HttpResponse.json({
      skills_available: 0,
      skills_included: [],
      skills_enabled: false,
      active_skill: null,
    }),
  ),
  http.get("*/v1/buddy/opportunities", () =>
    HttpResponse.json({ opportunities: [] }),
  ),
  http.get("*/v1/worktrees", () =>
    HttpResponse.json({
      project_hash: "test",
      source_workspace_root: "/tmp/refact-test",
      worktrees: [],
    }),
  ),
];

const baseConfig = {
  host: "vscode" as const,
  lspPort: 8001,
  apiKey: "test",
  themeProps: {},
};

function chatHistoryItem({
  id,
  content,
  buddy,
}: {
  id: string;
  content: string;
  buddy: boolean;
}): ChatHistoryItem {
  return {
    id,
    title: buddy ? "Buddy Investigation" : "Normal Chat",
    model: "",
    mode: buddy ? "buddy" : "agent",
    tool_use: "agent",
    messages: [
      {
        role: "assistant",
        content,
        message_id: `${id}-message`,
      },
    ],
    boost_reasoning: false,
    context_tokens_cap: undefined,
    include_project_info: true,
    increase_max_tokens: false,
    last_user_message_id: "",
    createdAt: "2024-01-01T00:00:00Z",
    updatedAt: "2024-01-01T00:00:00Z",
    buddy_meta: buddy
      ? {
          is_buddy_chat: true,
          buddy_chat_kind: "investigation",
          workflow_id: null,
        }
      : undefined,
  };
}

function renderChatPage(item: ChatHistoryItem) {
  server.use(...appHandlers);
  const store = setUpStore({
    config: baseConfig,
    current_project: {
      name: "refact-test",
      workspaceRoots: ["/tmp/refact-test"],
    },
    pages: [{ name: "chat" }],
  });
  store.dispatch(setBackendStatus({ status: "online" }));
  setProjectStorageNamespaceFromProjectInfo({
    workspaceRoots: ["/tmp/refact-test"],
    projectName: "refact-test",
  });
  store.dispatch(restoreChat(item));

  return render(<InnerApp />, { store });
}

afterEach(() => {
  localStorage.clear();
  sessionStorage.clear();
  setProjectStorageNamespace(undefined);
  vi.clearAllMocks();
});

describe("App buddy chat page rendering", () => {
  it("renders the current buddy chat with the single Chat container", async () => {
    renderChatPage(
      chatHistoryItem({
        id: "buddy-chat-1",
        content: "Buddy investigation transcript squeak",
        buddy: true,
      }),
    );

    const singleChat = await screen.findByTestId("single-chat");

    expect(singleChat).toHaveAttribute("data-chat-id", "buddy-chat-1");
    expect(singleChat).toHaveTextContent("Buddy investigation transcript squeak");
    await waitFor(() => {
      expect(screen.queryByTestId("split-chat-layout")).not.toBeInTheDocument();
    });
  });

  it("keeps normal current chats on the split layout", async () => {
    renderChatPage(
      chatHistoryItem({
        id: "normal-chat-1",
        content: "Normal transcript stays pane routed",
        buddy: false,
      }),
    );

    expect(await screen.findByTestId("split-chat-layout")).toBeInTheDocument();
    expect(screen.queryByTestId("single-chat")).not.toBeInTheDocument();
  });
});
