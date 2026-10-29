import { http, HttpResponse } from "msw";
import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "../../../utils/test-utils";
import { server } from "../../../utils/mockServer";
import { BuddyRecentChats, CHATS_PAGE_SIZE } from "../BuddyRecentChats";
import { readBuddyPersistentState, resetBuddyPersistentState } from "../ui";
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

afterEach(() => resetBuddyPersistentState());

describe("BuddyRecentChats", () => {
  it("pages the Activity chats list ten rows at a time", async () => {
    const entries = Array.from({ length: 25 }, (_, index) =>
      makeConversation({
        id: `chat-${index}`,
        title: `Chat ${index}`,
        updated_at: new Date(Date.UTC(2026, 0, 1, 0, 59 - index)).toISOString(),
      }),
    );
    server.use(
      http.get("*/v1/buddy/conversations", () => HttpResponse.json(entries)),
    );

    const { user } = render(<BuddyRecentChats initialFilter="all" />, {
      preloadedState: CONFIG_STATE,
    });

    await screen.findByRole("list");
    expect(CHATS_PAGE_SIZE).toBe(10);
    expect(screen.getAllByRole("listitem")).toHaveLength(10);
    await user.click(screen.getByRole("button", { name: "Show more" }));
    expect(screen.getAllByRole("listitem")).toHaveLength(20);
    await user.click(screen.getByRole("button", { name: "Show more" }));
    expect(screen.getAllByRole("listitem")).toHaveLength(25);
    expect(screen.queryByRole("button", { name: "Show more" })).toBeNull();
  });

  it("renders capped workflow history as human-readable entry-counted lists", async () => {
    const entries = Array.from({ length: 60 }, (_, index) => {
      const workflow = Math.floor(index / 12);
      const ids = [
        "buddy_error_detective",
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
    expect(screen.getAllByRole("listitem")).toHaveLength(CHATS_PAGE_SIZE);
    expect(document.body.textContent).toContain("1 entry");
    expect(screen.getAllByText("2 chats").length).toBeGreaterThan(0);
    expect(document.body.textContent).not.toMatch(/\b[a-z]+(?: [a-z]+){2,} \(/);
    expect(document.body.textContent).not.toMatch(
      /\b1 entries\b|\b1 messages\b/,
    );
    expect(document.body.textContent).not.toContain("1 message");
    expect(
      screen.getByRole("button", { name: "Open Error Detective report 2" }),
    ).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Show more" }));
    expect(screen.getAllByRole("listitem")).toHaveLength(2 * CHATS_PAGE_SIZE);
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

    const { store, user } = render(<BuddyRecentChats initialFilter="chat" />, {
      preloadedState: CONFIG_STATE,
    });

    expect(
      await screen.findByRole("heading", { name: "Conversations" }),
    ).toBeInTheDocument();
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
      badge: "Workflow",
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
      screen.getByRole("button", { name: "Open Error Detective" }).textContent,
    ).toContain("Error Detective");
    expect(
      screen
        .getByRole("button", { name: "Open Error Detective" })
        .textContent?.match(/Error Detective/g),
    ).toHaveLength(1);
    expect(
      screen
        .getByRole("button", { name: "Open Error Detective" })
        .querySelectorAll('[class*="entryBadge"]'),
    ).toHaveLength(0);
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

    render(<BuddyRecentChats initialFilter="all" />, {
      preloadedState: CONFIG_STATE,
    });

    expect(await screen.findByText(/5 entries/)).toBeInTheDocument();
    expect(screen.queryByText(/5 messages/)).not.toBeInTheDocument();
  });

  it("renders distinct knowledge workflow titles exactly once without a Knowledge chip", async () => {
    const entries = [
      makeConversation({
        id: "kg_enrich",
        kind: "system",
        title: "Knowledge graph enrichment",
        status: "failed",
        message_count: 100,
        badge: "Knowledge",
      }),
      makeConversation({
        id: "kg_deprecate",
        kind: "system",
        title: "Knowledge cleanup",
        status: "failed",
        message_count: 100,
        badge: "Knowledge",
      }),
    ];
    server.use(
      http.get("*/v1/buddy/conversations", () => HttpResponse.json(entries)),
    );

    render(<BuddyRecentChats compact showFilters={false} />, {
      preloadedState: CONFIG_STATE,
    });

    const rows = await screen.findAllByRole("listitem");
    expect(rows).toHaveLength(2);
    expect(rows[0]).toHaveTextContent("Knowledge graph enrichment");
    expect(rows[1]).toHaveTextContent("Knowledge cleanup");
    for (const row of rows) {
      const rowText = row.textContent ?? "";
      expect(rowText.match(/Knowledge/g)).toHaveLength(1);
      expect(row.querySelectorAll('[class*="entryBadge"]')).toHaveLength(0);
    }
  });

  it("renders API-shaped workflow history with one name, visible failure status, and matching actions", async () => {
    const entries = [
      makeConversation({
        id: "memo_extraction",
        kind: "system",
        title: "Memo extraction",
        created_at: "2026-09-24T14:14:29.963146380+00:00",
        updated_at: "2026-09-24T14:14:29.963146380+00:00",
        status: "failed",
        message_count: 100,
        icon: "🧠",
        badge: "Memo",
      }),
      makeConversation({
        id: "kg_enrich",
        kind: "system",
        title: "Knowledge graph enrichment",
        created_at: "2026-09-24T06:54:38.312631790+00:00",
        updated_at: "2026-09-24T06:54:38.312631790+00:00",
        status: "failed",
        message_count: 100,
        icon: "📚",
        badge: "Knowledge",
      }),
      makeConversation({
        id: "kg_deprecate",
        kind: "system",
        title: "Knowledge cleanup",
        created_at: "2026-09-24T05:54:38.312631790+00:00",
        updated_at: "2026-09-24T05:54:38.312631790+00:00",
        status: "failed",
        message_count: 100,
        icon: "📚",
        badge: "Knowledge",
      }),
      makeConversation({
        id: "commit_message",
        kind: "workflow",
        title: "Commit message generation",
        created_at: "2026-09-22T18:50:59.124159204+00:00",
        updated_at: "2026-09-22T18:50:59.124159204+00:00",
        status: "failed",
        message_count: 100,
        icon: "🔄",
        badge: "Commit Msg",
      }),
      makeConversation({
        id: "55b749c2-540e-45f2-99ce-d852698a4f2a",
        kind: "system",
        title: "Privacy observation degraded",
        created_at: "2026-09-21T12:51:39.536239200+00:00",
        updated_at: "2026-09-21T12:51:39.536239200+00:00",
        status: "active",
        message_count: 1,
        icon: "🔄",
        badge: null,
      }),
      makeConversation({
        id: "title_generation",
        kind: "workflow",
        title: "Title generation",
        created_at: "2026-06-03T05:55:03.452956309+00:00",
        updated_at: "2026-06-03T05:55:03.452956309+00:00",
        status: "completed",
        message_count: 100,
        icon: "🔄",
        badge: null,
      }),
    ];
    server.use(
      http.get("*/v1/buddy/conversations", () => HttpResponse.json(entries)),
      http.get("*/v1/trajectories/:id", ({ params }) =>
        HttpResponse.json({
          id: String(params.id),
          title: "Memo",
          created_at: entries[0].created_at,
          updated_at: entries[0].updated_at,
          model: "",
          mode: "buddy",
          tool_use: "agent",
          messages: [],
        }),
      ),
    );

    const { store, user } = render(
      <BuddyRecentChats compact showFilters={false} />,
      { preloadedState: CONFIG_STATE },
    );

    const rows = await screen.findAllByRole("listitem");
    expect(rows).toHaveLength(entries.length);
    for (const [index, entry] of entries.entries()) {
      const rowText = rows[index].textContent ?? "";
      const name = entry.title;
      const nameOccurrences =
        rowText.toLowerCase().split(name.toLowerCase()).length - 1;
      expect(nameOccurrences, `row ${index} name ${name}: ${rowText}`).toBe(1);
      if (entry.status === "active") {
        expect(rows[index].querySelector("button")).toBeNull();
        expect(rows[index]).toHaveTextContent("1 message");
      } else {
        expect(
          screen.getByRole("button", { name: `Open ${name}` }),
        ).toBeInTheDocument();
      }
      if (entry.status === "failed") {
        expect(rows[index]).toHaveTextContent("Failed");
        expect(rows[index]).toHaveTextContent("100 entries");
      } else if (entry.status === "completed") {
        expect(rows[index]).toHaveTextContent("100 entries");
        expect(rows[index]).not.toHaveTextContent("Failed");
      }
    }
    expect(screen.getAllByText("Failed")).toHaveLength(4);
    expect(rows[3]).toHaveTextContent("Commit Msg");
    expect(rows[0]).toHaveTextContent("Memo extraction");
    expect(rows[0].textContent?.match(/Memo/g)).toHaveLength(1);
    expect(rows[1]).toHaveTextContent("Knowledge graph enrichment");
    expect(rows[2]).toHaveTextContent("Knowledge cleanup");
    await user.click(
      screen.getByRole("button", { name: "Open Memo extraction" }),
    );
    await waitFor(() => {
      expect(store.getState().chat.current_thread_id).toBe("memo_extraction");
      expect(
        store.getState().chat.threads.memo_extraction?.thread.buddy_meta,
      ).toEqual({
        is_buddy_chat: true,
        buddy_chat_kind: "workflow",
        workflow_id: "memo_extraction",
      });
    });
  });
});

describe("BuddyRecentChats filters", () => {
  const mixed = [
    makeConversation({ id: "human", kind: "chat", title: "Human chat" }),
    makeConversation({
      id: "buddy_memory_garden",
      kind: "workflow",
      title: "Memory garden",
      badge: "Memory garden",
    }),
    makeConversation({ id: "setup-1", kind: "setup", title: "Setup run" }),
  ];

  it("defaults Activity to background chats, follows the filter heading and persists it", async () => {
    server.use(
      http.get("*/v1/buddy/conversations", () => HttpResponse.json(mixed)),
    );
    const { user, unmount } = render(<BuddyRecentChats />, {
      preloadedState: CONFIG_STATE,
    });
    expect(
      await screen.findByRole("heading", { name: "Background chats" }),
    ).toBeInTheDocument();
    expect(await screen.findByText("Memory garden")).toBeInTheDocument();
    expect(screen.queryByText("Human chat")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "New chat" })).toBeEnabled();

    await user.click(screen.getByRole("radio", { name: "Chats" }));
    expect(
      screen.getByRole("heading", { name: "Conversations" }),
    ).toBeInTheDocument();
    expect(screen.getByText("Human chat")).toBeInTheDocument();
    unmount();

    render(<BuddyRecentChats />, { preloadedState: CONFIG_STATE });
    expect(
      await screen.findByRole("heading", { name: "Conversations" }),
    ).toBeInTheDocument();
  });

  it("shows a filter-specific empty state without template leaks", async () => {
    server.use(
      http.get("*/v1/buddy/conversations", () => HttpResponse.json([mixed[0]])),
    );
    const { user } = render(<BuddyRecentChats />, {
      preloadedState: CONFIG_STATE,
    });
    expect(
      await screen.findByText("No background chats yet"),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("radio", { name: "Setup" }));
    expect(screen.getByText("No setup chats yet")).toBeInTheDocument();
    expect(document.body.textContent).not.toMatch(/No \w+ entries/);
  });

  it("compact Home list shows recent conversations and View all preselects Conversations", async () => {
    server.use(
      http.get("*/v1/buddy/conversations", () => HttpResponse.json(mixed)),
    );
    const onViewAll = vi.fn();
    const { user } = render(
      <BuddyRecentChats
        compact
        maxItems={3}
        initialFilter="chat"
        onViewAll={onViewAll}
      />,
      { preloadedState: CONFIG_STATE },
    );
    expect(
      await screen.findByRole("heading", { name: "Recent conversations" }),
    ).toBeInTheDocument();
    expect(await screen.findByText("Human chat")).toBeInTheDocument();
    expect(screen.queryByText("Memory garden")).not.toBeInTheDocument();
    expect(screen.queryByRole("radiogroup")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "View all" }));
    expect(onViewAll).toHaveBeenCalledTimes(1);
    expect(readBuddyPersistentState("buddy.activity.chatsFilter")).toBe("chat");
  });

  it("shows one-line error with retry when conversations fail to load", async () => {
    let calls = 0;
    server.use(
      http.get("*/v1/buddy/conversations", () => {
        calls += 1;
        return calls === 1
          ? HttpResponse.json({}, { status: 500 })
          : HttpResponse.json(mixed);
      }),
    );
    const { user } = render(<BuddyRecentChats initialFilter="chat" />, {
      preloadedState: CONFIG_STATE,
    });
    expect(await screen.findByText("Couldn't load chats.")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Retry" }));
    expect(await screen.findByText("Human chat")).toBeInTheDocument();
  });
});
