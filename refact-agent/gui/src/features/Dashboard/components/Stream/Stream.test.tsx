import type {
  Assignment,
  OrganizationOperation,
} from "../../../../services/refact/organization";
import {
  OrganizationContext,
  useOrganizationData,
} from "../../hooks/useOrganization";
import { organizationTestHandlers } from "../../organizationTestHandlers";
import { http, HttpResponse } from "msw";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { server } from "../../../../utils/mockServer";
import {
  render as renderWithoutOrganization,
  screen,
  fireEvent,
  waitFor,
  within,
} from "../../../../utils/test-utils";
import { StreamSection } from "./StreamSection";
import { StreamRow } from "./StreamRow";
import { RowPeek } from "./RowPeek";
import { ThreadRail } from "./ThreadRail";
import type { StreamItem } from "../../streamSelectors";
import type { ChatHistoryItem } from "../../../History/historySlice";

const NOW = Date.now();

function makeChat(
  partial: Partial<ChatHistoryItem> & { id: string },
): ChatHistoryItem {
  return {
    title: `chat ${partial.id}`,
    model: "gpt-5",
    mode: "AGENT",
    tool_use: "agent",
    read: true,
    createdAt: new Date(NOW).toISOString(),
    updatedAt: new Date(NOW).toISOString(),
    messages: [],
    new_chat_suggested: { wasSuggested: false },
    ...partial,
  } as ChatHistoryItem;
}

function preloadedStateWith(chats: ChatHistoryItem[]) {
  const byId: Record<string, ChatHistoryItem> = {};
  for (const chat of chats) byId[chat.id] = chat;
  return {
    history: {
      chats: byId,
      isLoading: false,
      loadError: null,
      pagination: {
        cursor: null,
        hasMore: false,
        totalCount: chats.length,
        generation: 1,
      },
    },
  };
}

function trajectoryMeta(id: string, title: string) {
  return {
    id,
    title,
    created_at: new Date(NOW).toISOString(),
    updated_at: new Date(NOW - 60_000).toISOString(),
    model: "gpt-5",
    mode: "agent",
    message_count: 1,
    total_lines_added: 0,
    total_lines_removed: 0,
    tasks_total: 0,
    tasks_done: 0,
    tasks_failed: 0,
  };
}

beforeEach(() => server.use(...organizationTestHandlers));

function OrganizationTestProvider({ children }: { children: React.ReactNode }) {
  const organization = useOrganizationData();
  return (
    <OrganizationContext.Provider value={organization}>
      {children}
    </OrganizationContext.Provider>
  );
}

function render(
  ui: Parameters<typeof renderWithoutOrganization>[0],
  options?: Parameters<typeof renderWithoutOrganization>[1],
) {
  return renderWithoutOrganization(
    <OrganizationTestProvider>{ui}</OrganizationTestProvider>,
    options,
  );
}

const ALL_FILTER = { kind: "all" as const, query: "" };

describe("StreamSection", () => {
  it("persists manual tag locks independently and exposes Buddy resumption", async () => {
    let assignment: Assignment = {
      item: { kind: "chat", id: "a" },
      topic_id: null,
      tag_ids: [],
      topic_locked: false,
      tags_locked: false,
    };
    let revision = 1;
    const operations: OrganizationOperation[] = [];
    server.use(
      http.get("*/v1/organization/taxonomy", ({ request }) =>
        HttpResponse.json({
          revision,
          items:
            new URL(request.url).searchParams.get("kind") === "tags"
              ? [{ id: "tag", name: "Review", archived: false }]
              : [],
          next_cursor: null,
        }),
      ),
      http.post("*/v1/organization/items", () =>
        HttpResponse.json({ revision, assignments: [assignment] }),
      ),
      http.post("*/v1/organization/apply", async ({ request }) => {
        const body = (await request.json()) as {
          operations: OrganizationOperation[];
          expected_revision: number;
        };
        expect(body.expected_revision).toBe(revision);
        operations.push(...body.operations);
        const operation = body.operations[0];
        if (operation.op === "assign_item") assignment = operation;
        revision++;
        return HttpResponse.json({ revision, results: [{ op_index: 0 }] });
      }),
    );
    const { user } = render(
      <StreamSection
        filter={ALL_FILTER}
        onOpenChat={vi.fn()}
        onOpenTask={vi.fn()}
      />,
      {
        preloadedState: preloadedStateWith([
          makeChat({ id: "a", title: "Editable" }),
        ]),
      },
    );
    fireEvent.click(screen.getByTestId("stream-expand-a"));
    await user.click(screen.getByRole("button", { name: "Topic & tags" }));
    await waitFor(() =>
      expect(screen.getByRole("checkbox", { name: "Review" })).toBeEnabled(),
    );
    await user.click(screen.getByRole("checkbox", { name: "Review" }));
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Let Buddy manage tags" }),
      ).toBeEnabled(),
    );
    expect(operations[0]).toMatchObject({
      op: "assign_item",
      item: { kind: "chat", id: "a" },
      tag_ids: ["tag"],
      tags_locked: true,
      topic_locked: false,
    });
    await user.click(
      screen.getByRole("button", { name: "Let Buddy manage tags" }),
    );
    await waitFor(() => expect(operations).toHaveLength(2));
    expect(operations[1]).toMatchObject({
      tags_locked: false,
      tag_ids: ["tag"],
      topic_locked: false,
    });
  });
  it("filters real loaded assignments while keeping unclassified history and pagination honest", async () => {
    server.use(
      http.get("*/v1/organization/taxonomy", ({ request }) =>
        HttpResponse.json({
          revision: 1,
          items:
            new URL(request.url).searchParams.get("kind") === "topics"
              ? [{ id: "topic", name: "Interface", archived: false }]
              : [{ id: "tag", name: "Review", archived: false }],
          next_cursor: null,
        }),
      ),
      http.post("*/v1/organization/items", () =>
        HttpResponse.json({
          revision: 1,
          assignments: [
            {
              item: { kind: "chat", id: "a" },
              topic_id: "topic",
              tag_ids: ["tag"],
              topic_locked: false,
              tags_locked: false,
            },
            {
              item: { kind: "chat", id: "b" },
              topic_id: null,
              tag_ids: [],
              topic_locked: false,
              tags_locked: false,
            },
          ],
        }),
      ),
    );
    const state = preloadedStateWith([
      makeChat({ id: "a", title: "Classified" }),
      makeChat({ id: "b", title: "Unclassified" }),
    ]);
    const { rerender } = render(
      <StreamSection
        filter={ALL_FILTER}
        onOpenChat={vi.fn()}
        onOpenTask={vi.fn()}
      />,
      {
        preloadedState: {
          ...state,
          history: {
            ...state.history,
            pagination: {
              ...state.history.pagination,
              hasMore: true,
              cursor: "older",
            },
          },
        },
      },
    );
    await waitFor(() =>
      expect(screen.getByText("Interface")).toBeInTheDocument(),
    );
    expect(screen.getByText("Unclassified")).toBeInTheDocument();
    rerender(
      <StreamSection
        filter={{ ...ALL_FILTER, topicId: "topic", tagIds: ["tag"] }}
        onOpenChat={vi.fn()}
        onOpenTask={vi.fn()}
      />,
    );
    expect(screen.getByText("Classified")).toBeInTheDocument();
    expect(screen.queryByText("Unclassified")).not.toBeInTheDocument();
    expect(
      screen.getByText(/Filtering loaded history only/),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Load older chats" }),
    ).toBeInTheDocument();
    rerender(
      <StreamSection
        filter={{ ...ALL_FILTER, topicId: "unclassified" }}
        onOpenChat={vi.fn()}
        onOpenTask={vi.fn()}
      />,
    );
    expect(screen.getByText("Unclassified")).toBeInTheDocument();
    expect(screen.queryByText("Classified")).not.toBeInTheDocument();
  });
  it("keeps a valid root filterable and editable beside a forbidden orphan", async () => {
    server.use(
      http.get("*/v1/organization/taxonomy", ({ request }) =>
        HttpResponse.json({
          revision: 1,
          items:
            new URL(request.url).searchParams.get("kind") === "topics"
              ? [{ id: "topic", name: "Interface", archived: false }]
              : [{ id: "tag", name: "Review", archived: false }],
          next_cursor: null,
        }),
      ),
      http.post("*/v1/organization/items", async ({ request }) => {
        const { items } = (await request.json()) as {
          items: { kind: string; id: string }[];
        };
        if (items.some((item) => item.id === "orphan"))
          return new HttpResponse(null, { status: 403 });
        return HttpResponse.json({
          revision: 1,
          assignments: items.map((item) => ({
            item,
            topic_id: "topic",
            tag_ids: ["tag"],
            topic_locked: false,
            tags_locked: false,
          })),
        });
      }),
    );
    const { rerender, user } = render(
      <StreamSection
        filter={ALL_FILTER}
        onOpenChat={vi.fn()}
        onOpenTask={vi.fn()}
      />,
      {
        preloadedState: preloadedStateWith([
          makeChat({ id: "root", title: "Valid root" }),
          makeChat({ id: "orphan", title: "Legacy orphan" }),
        ]),
      },
    );
    await waitFor(() =>
      expect(screen.getByText("Interface")).toBeInTheDocument(),
    );
    expect(screen.getByText("Review")).toBeInTheDocument();
    expect(screen.getByText("Legacy orphan")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Retry organization" }),
    ).not.toBeInTheDocument();
    fireEvent.click(screen.getByTestId("stream-expand-root"));
    await user.click(screen.getByRole("button", { name: "Topic & tags" }));
    expect(screen.getByRole("combobox", { name: "Item topic" })).toBeEnabled();
    await user.keyboard("{Escape}");
    fireEvent.click(screen.getByTestId("stream-expand-orphan"));
    await user.click(screen.getByRole("button", { name: "Topic & tags" }));
    expect(screen.getByRole("combobox", { name: "Item topic" })).toBeDisabled();
    expect(
      screen.getByRole("combobox", { name: "Item topic" }),
    ).toHaveTextContent("Unknown");
    expect(
      screen.getByRole("button", { name: "Create and assign" }),
    ).toBeDisabled();
    await user.keyboard("{Escape}");
    rerender(
      <StreamSection
        filter={{ ...ALL_FILTER, topicId: "topic", tagIds: ["tag"] }}
        onOpenChat={vi.fn()}
        onOpenTask={vi.fn()}
      />,
    );
    expect(screen.getByText("Valid root")).toBeInTheDocument();
    expect(screen.queryByText("Legacy orphan")).not.toBeInTheDocument();
    rerender(
      <StreamSection
        filter={{ ...ALL_FILTER, topicId: "unclassified" }}
        onOpenChat={vi.fn()}
        onOpenTask={vi.fn()}
      />,
    );
    expect(screen.queryByText("Legacy orphan")).not.toBeInTheDocument();
    expect(screen.queryByText("Valid root")).not.toBeInTheDocument();
  });
  it("keeps history visible on organization failure even with topic filters", async () => {
    server.use(
      http.get(
        "*/v1/organization/taxonomy",
        () => new HttpResponse(null, { status: 503 }),
      ),
    );
    render(
      <StreamSection
        filter={{ ...ALL_FILTER, topicId: "missing" }}
        onOpenChat={vi.fn()}
        onOpenTask={vi.fn()}
      />,
      {
        preloadedState: preloadedStateWith([
          makeChat({ id: "a", title: "Still available" }),
        ]),
      },
    );
    await waitFor(() =>
      expect(screen.getByText("Still available")).toBeInTheDocument(),
    );
    expect(
      screen.getByRole("button", { name: "Retry organization" }),
    ).toBeInTheDocument();
  });
  it("shows truthful coarse chat activity separately from family aggregate status", () => {
    render(
      <StreamSection
        filter={ALL_FILTER}
        onOpenChat={vi.fn()}
        onOpenTask={vi.fn()}
      />,
      {
        preloadedState: preloadedStateWith([
          makeChat({ id: "root", title: "Root", session_state: "completed" }),
          makeChat({
            id: "child",
            title: "Child",
            parent_id: "root",
            session_state: "executing_tools",
          }),
        ]),
      },
    );
    expect(
      within(screen.getByTestId("stream-row-root")).getByLabelText("Completed"),
    ).toBeInTheDocument();
    fireEvent.click(
      screen.getByRole("button", { name: "Toggle thread family for Root" }),
    );
    expect(
      within(screen.getByTestId("stream-rail-row-child")).getByLabelText(
        "Running tools",
      ),
    ).toBeInTheDocument();
    expect(screen.queryByText("Waiting for subagents")).not.toBeInTheDocument();
  });

  it("toggles the thread rail from the family pill", () => {
    render(
      <StreamSection
        filter={ALL_FILTER}
        onOpenChat={vi.fn()}
        onOpenTask={vi.fn()}
      />,
      {
        preloadedState: preloadedStateWith([
          makeChat({ id: "root", title: "Root chat" }),
          makeChat({
            id: "child",
            title: "Child chat",
            parent_id: "root",
            link_type: "subchat",
          }),
        ]),
      },
    );

    expect(screen.queryByTestId("stream-rail-row-child")).toBeNull();
    fireEvent.click(
      screen.getByRole("button", {
        name: "Toggle thread family for Root chat",
      }),
    );
    expect(screen.getByTestId("stream-rail-row-child")).toBeInTheDocument();
    expect(screen.getByText("Child chat")).toBeInTheDocument();
  });

  it("opens the metadata peek from a row click", () => {
    render(
      <StreamSection
        filter={ALL_FILTER}
        onOpenChat={vi.fn()}
        onOpenTask={vi.fn()}
      />,
      {
        preloadedState: preloadedStateWith([
          makeChat({
            id: "a",
            title: "Peekable",
            model: "gpt-5",
            total_tokens: 1000,
            total_cache_read_tokens: 500,
          }),
        ]),
      },
    );

    expect(screen.queryByTestId("stream-peek-a")).toBeNull();
    fireEvent.click(screen.getByTestId("stream-expand-a"));
    const peek = screen.getByTestId("stream-peek-a");
    expect(peek).toBeInTheDocument();
    expect(screen.getByText("Model")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Open" })).toBeInTheDocument();
  });

  it("gates row deletion behind the DeletePopover confirm", () => {
    render(
      <StreamSection
        filter={ALL_FILTER}
        onOpenChat={vi.fn()}
        onOpenTask={vi.fn()}
      />,
      {
        preloadedState: preloadedStateWith([
          makeChat({ id: "a", title: "Deletable" }),
        ]),
      },
    );

    fireEvent.click(screen.getByTestId("stream-expand-a"));
    fireEvent.click(screen.getByRole("button", { name: "Delete Deletable" }));
    expect(screen.getByText("Destructive action")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.getByTestId("stream-peek-a")).toBeInTheDocument();
  });

  it("offers pagination when older chats are available", () => {
    const preloadedState = preloadedStateWith([
      makeChat({ id: "a", title: "Current" }),
    ]);
    render(
      <StreamSection
        filter={ALL_FILTER}
        onOpenChat={vi.fn()}
        onOpenTask={vi.fn()}
      />,
      {
        preloadedState: {
          ...preloadedState,
          history: {
            ...preloadedState.history,
            pagination: {
              cursor: "older-page",
              hasMore: true,
              totalCount: 51,
              generation: 1,
            },
          },
        },
      },
    );

    expect(
      screen.getByRole("button", { name: "Load older chats" }),
    ).toBeInTheDocument();
  });

  it("keeps loading older chats available when the current search is empty", () => {
    const preloadedState = preloadedStateWith([
      makeChat({ id: "a", title: "Current" }),
    ]);
    render(
      <StreamSection
        filter={{ kind: "chat", query: "older" }}
        onOpenChat={vi.fn()}
        onOpenTask={vi.fn()}
      />,
      {
        preloadedState: {
          ...preloadedState,
          history: {
            ...preloadedState.history,
            pagination: {
              cursor: "older-page",
              hasMore: true,
              totalCount: 51,
              generation: 1,
            },
          },
        },
      },
    );

    expect(
      screen.getByText("No matches in loaded history."),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Load older chats" }),
    ).toBeInTheDocument();
  });

  it("loads the next history page when requested", async () => {
    const preloadedState = preloadedStateWith([
      makeChat({ id: "current", title: "Current" }),
    ]);
    let requestedCursor: string | null = null;
    server.use(
      http.get("*/v1/trajectories", ({ request }) => {
        requestedCursor = new URL(request.url).searchParams.get("cursor");
        return HttpResponse.json({
          items: [trajectoryMeta("older", "Older flat chat")],
          next_cursor: null,
          has_more: false,
          total_count: 2,
        });
      }),
    );

    render(
      <StreamSection
        filter={ALL_FILTER}
        onOpenChat={vi.fn()}
        onOpenTask={vi.fn()}
      />,
      {
        preloadedState: {
          ...preloadedState,
          history: {
            ...preloadedState.history,
            pagination: {
              cursor: "older-page",
              hasMore: true,
              totalCount: 2,
              generation: 1,
            },
          },
        },
      },
    );

    fireEvent.click(screen.getByRole("button", { name: "Load older chats" }));

    await waitFor(() => {
      expect(requestedCursor).toBe("older-page");
      expect(screen.getByText("Older flat chat")).toBeInTheDocument();
    });
  });
});

function makeStreamItem(kind: StreamItem["kind"]): StreamItem {
  return {
    kind,
    id: "content-id",
    title: "Content title",
    updatedAtMs: NOW,
    createdAtMs: null,
    status: "idle",
    mode: null,
    model: null,
    messageCount: null,
    diff: null,
    branch: null,
    costUsd: null,
    totalTokens: null,
    cacheReadTokens: null,
    familyChildCount: 1,
    cardsDone: null,
    cardsTotal: null,
    cardsFailed: null,
    agentsActive: null,
    linkedChats: null,
  };
}

describe("Stream native content links", () => {
  it.each(["chat", "task"] as const)(
    "provides token-free %s title and Open links with one primary callback",
    (kind) => {
      const previousUrl = window.location.href;
      window.history.replaceState({}, "", "/gui?token=secret#/dashboard");
      try {
        const onOpen = vi.fn();
        const item = makeStreamItem(kind);
        render(
          <>
            <StreamRow
              item={item}
              index={0}
              isPeekOpen={false}
              isFamilyOpen={false}
              onOpen={onOpen}
              onTogglePeek={vi.fn()}
              onToggleFamily={vi.fn()}
            />
            <RowPeek item={item} onOpen={onOpen} onDelete={vi.fn()} />
          </>,
        );

        for (const name of [item.title, "Open"]) {
          const link = screen.getByRole("link", { name });
          expect(link).toHaveAttribute("href", `/gui#/${kind}/${item.id}`);
          expect(link.querySelector("button, [role=button]")).toBeNull();
          onOpen.mockClear();
          expect(fireEvent.click(link)).toBe(false);
          expect(onOpen).toHaveBeenCalledTimes(1);
          onOpen.mockClear();
          for (const modifiers of [
            { ctrlKey: true },
            { metaKey: true },
            { shiftKey: true },
            { altKey: true },
            { button: 1 },
          ]) {
            expect(fireEvent.click(link, modifiers)).toBe(true);
          }
          expect(
            fireEvent(
              link,
              new MouseEvent("auxclick", {
                button: 1,
                bubbles: true,
                cancelable: true,
              }),
            ),
          ).toBe(true);
          link.setAttribute("target", "_blank");
          expect(fireEvent.click(link)).toBe(true);
          link.removeAttribute("target");
          fireEvent.keyDown(link, { key: "Enter", ctrlKey: true });
          expect(onOpen).not.toHaveBeenCalled();
        }
        fireEvent.click(screen.getByTestId(`stream-row-${item.id}`));
        expect(onOpen).toHaveBeenCalledTimes(1);
        expect(onOpen).toHaveBeenCalledWith(item);
      } finally {
        window.history.replaceState({}, "", previousUrl);
      }
    },
  );

  it("keeps family and peek controls independent from title navigation", () => {
    const onOpen = vi.fn();
    const onToggleFamily = vi.fn();
    const onTogglePeek = vi.fn();
    const item = makeStreamItem("chat");
    render(
      <StreamRow
        item={item}
        index={0}
        isPeekOpen={false}
        isFamilyOpen={false}
        onOpen={onOpen}
        onTogglePeek={onTogglePeek}
        onToggleFamily={onToggleFamily}
      />,
    );
    const family = screen.getByRole("button", {
      name: `Toggle thread family for ${item.title}`,
    });
    expect(family.closest("a")).toBeNull();
    fireEvent.click(family);
    expect(onToggleFamily).toHaveBeenCalledExactlyOnceWith(item.id);
    const expand = screen.getByTestId(`stream-expand-${item.id}`);
    expect(expand.closest("a")).toBeNull();
    fireEvent.click(expand);
    expect(onTogglePeek).toHaveBeenCalledExactlyOnceWith(item.id);
    fireEvent.keyDown(expand, { key: "Enter" });
    expect(onOpen).not.toHaveBeenCalled();
  });

  it("gives expanded thread titles native links without bubbling to the row", () => {
    const onOpenChat = vi.fn();
    render(<ThreadRail rootId="root" onOpenChat={onOpenChat} />, {
      preloadedState: preloadedStateWith([
        makeChat({ id: "root", title: "Root" }),
        makeChat({ id: "child", title: "Child", parent_id: "root" }),
      ]),
    });
    const link = screen.getByRole("link", { name: "Child" });
    expect(link).toHaveAttribute(
      "href",
      `${window.location.pathname}#/chat/child`,
    );
    expect(fireEvent.click(link, { metaKey: true })).toBe(true);
    expect(fireEvent.click(link, { ctrlKey: true })).toBe(true);
    expect(fireEvent.click(link, { button: 1 })).toBe(true);
    link.setAttribute("target", "_blank");
    expect(fireEvent.click(link)).toBe(true);
    link.removeAttribute("target");
    fireEvent.keyDown(link, { key: "Enter" });
    expect(onOpenChat).not.toHaveBeenCalled();
    expect(fireEvent.click(link)).toBe(false);
    expect(onOpenChat).toHaveBeenCalledExactlyOnceWith("child");
  });

  it.each(["ide", "vscode", "jetbrains"] as const)(
    "preserves original %s title and Open elements",
    (host) => {
      const onOpen = vi.fn();
      const onOpenChat = vi.fn();
      const item = makeStreamItem("chat");
      render(
        <>
          <StreamRow
            item={item}
            index={0}
            isPeekOpen={false}
            isFamilyOpen={false}
            onOpen={onOpen}
            onTogglePeek={vi.fn()}
            onToggleFamily={vi.fn()}
          />
          <RowPeek item={item} onOpen={onOpen} onDelete={vi.fn()} />
          <ThreadRail rootId="root" onOpenChat={onOpenChat} />
        </>,
        {
          preloadedState: {
            ...preloadedStateWith([
              makeChat({ id: "root" }),
              makeChat({ id: "child", title: "Child", parent_id: "root" }),
            ]),
            config: { host, lspPort: 8001, themeProps: {} },
          },
        },
      );
      expect(screen.queryByRole("link")).toBeNull();
      fireEvent.click(screen.getByText(item.title), { ctrlKey: true });
      expect(onOpen).toHaveBeenCalledExactlyOnceWith(item);
      fireEvent.click(screen.getByRole("button", { name: "Open" }));
      expect(onOpen).toHaveBeenCalledTimes(2);
      fireEvent.click(screen.getByText("Child"), { metaKey: true });
      expect(onOpenChat).toHaveBeenCalledExactlyOnceWith("child");
    },
  );
});
