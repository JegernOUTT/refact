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
  it("renders capped workflow history as human-readable entry-counted lists", async () => {
    const entries = Array.from({ length: 60 }, (_, index) => {
      const workflow = Math.floor(index / 12);
      const ids = [
        "refact_error_detective",
        "buddy_pr_issue_matchmaker",
        "kg_deprecate",
        "privacy_observation_degraded",
        "commit_message",
      ];
      const names = [
        "Error Detective",
        "PR Matchmaker",
        "Knowledge",
        "Privacy observation degraded",
        "Commit message",
      ];
      const id = ids[workflow];
      const badge = names[workflow];
      const title =
        index % 12 < 2
          ? `${id.replace(/_/g, " ")} (${badge})`
          : `${badge} report ${index}`;
      return makeConversation({
        id: index % 12 < 2 ? id : `${id}-${index}`,
        kind: "workflow",
        title,
        badge,
        message_count: index === 0 ? 1 : 2,
        updated_at: new Date(Date.UTC(2026, 0, 1, 0, 59 - index)).toISOString(),
      });
    });
    server.use(
      http.get("*/v1/buddy/conversations", () => HttpResponse.json(entries)),
    );

    const { user } = render(<BuddyRecentChats compact showFilters={false} />, {
      preloadedState: CONFIG_STATE,
    });

    await screen.findByRole("list");
    expect(screen.getAllByRole("listitem")).toHaveLength(20);
    expect(document.body.textContent).toContain("1 entry");
    expect(screen.getAllByText("2 chats").length).toBeGreaterThan(0);
    expect(document.body.textContent).not.toMatch(/\b[a-z]+(?: [a-z]+){2,} \(/);
    expect(document.body.textContent).not.toMatch(
      /\b1 entries\b|\b1 messages\b/,
    );
    expect(document.body.textContent).not.toContain("1 message");
    expect(
      screen.getAllByRole("button", { name: /Open PR Matchmaker/ })[0],
    ).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Show more" }));
    expect(screen.getAllByRole("listitem")).toHaveLength(40);
  });

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
    expect(screen.getByText(/3 messages/)).toBeInTheDocument();
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

  it("uses the workflow badge only for the exact legacy title and preserves parenthetical titles", async () => {
    const legacy = makeConversation({
      id: "refact_error_detective",
      kind: "workflow",
      title: "refact error detective (Error Detective)",
      badge: "Error Detective",
      message_count: 1,
    });
    const parenthetical = makeConversation({
      id: "workflow-custom",
      kind: "workflow",
      title: "Fix login (urgent)",
      badge: "Fix Login",
      message_count: 1,
    });
    server.use(
      http.get("*/v1/buddy/conversations", () =>
        HttpResponse.json([legacy, parenthetical]),
      ),
    );

    render(<BuddyRecentChats compact showFilters={false} />, {
      preloadedState: CONFIG_STATE,
    });

    expect(
      await screen.findByRole("button", { name: "Open Error Detective" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Open Fix login (urgent)" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByText("refact error detective (Error Detective)"),
    ).not.toBeInTheDocument();
  });
  it("counts workflow-history run records as entries, never as messages", async () => {
    server.use(
      http.get("*/v1/buddy/conversations", () =>
        HttpResponse.json([
          makeConversation({
            id: "buddy_security_whisperer",
            kind: "system",
            title: "Security Whisperer",
            status: "failed",
            message_count: 5,
            icon: "🛡️",
            badge: "Security",
          }),
        ]),
      ),
    );

    render(<BuddyRecentChats title="RECENT CHATS" />, {
      preloadedState: CONFIG_STATE,
    });

    expect(await screen.findByText(/5 entries/)).toBeInTheDocument();
    expect(screen.queryByText(/5 messages/)).not.toBeInTheDocument();
  });
});
