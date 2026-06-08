import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import { render, screen, waitFor } from "../../../utils/test-utils";
import { server } from "../../../utils/mockServer";
import { BuddyRecentChats } from "../BuddyRecentChats";
import type { BuddyConversationEntry } from "../types";

const CONFIG_STATE = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "vscode" as const,
  },
};

function makeConversation(
  overrides?: Partial<BuddyConversationEntry>,
): BuddyConversationEntry {
  return {
    id: "buddy-chat-1",
    kind: "chat",
    title: "Gremlin triage chat",
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T01:00:00Z",
    status: "completed",
    message_count: 3,
    icon: "💬",
    badge: null,
    ...overrides,
  };
}

describe("BuddyRecentChats", () => {
  it("opens a recent chat row and navigates to chat", async () => {
    const entry = makeConversation();
    server.use(
      http.get("*/v1/buddy/conversations", () => HttpResponse.json([entry])),
      http.get("*/v1/trajectories/:id", ({ params }) =>
        HttpResponse.json({
          id: String(params.id),
          title: entry.title,
          created_at: entry.created_at,
          updated_at: entry.updated_at,
          model: "",
          mode: "buddy",
          tool_use: "agent",
          messages: [],
        }),
      ),
    );

    const { store, user } = render(<BuddyRecentChats title="RECENT CHATS" />, {
      preloadedState: CONFIG_STATE,
    });

    expect(await screen.findByText("RECENT CHATS")).toBeInTheDocument();
    await user.click(
      await screen.findByRole("button", { name: /Gremlin triage chat/i }),
    );

    await waitFor(() => {
      expect(store.getState().chat.current_thread_id).toBe(entry.id);
      expect(store.getState().chat.threads[entry.id]?.thread.title).toBe(
        entry.title,
      );
      expect(store.getState().pages.at(-1)).toEqual({ name: "chat" });
    });
  });
});
