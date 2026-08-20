import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent } from "../../../../utils/test-utils";
import { StreamSection } from "./StreamSection";
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

const ALL_FILTER = { kind: "all" as const, query: "" };

describe("StreamSection", () => {
  it("renders group headers and rows from the stream selectors", () => {
    render(
      <StreamSection
        filter={ALL_FILTER}
        onOpenChat={vi.fn()}
        onOpenTask={vi.fn()}
      />,
      {
        preloadedState: preloadedStateWith([
          makeChat({ id: "a", title: "Fix the toolbar" }),
        ]),
      },
    );

    expect(screen.getByTestId("stream-section")).toBeInTheDocument();
    expect(screen.getByTestId("stream-row-a")).toBeInTheDocument();
    expect(screen.getByText("Fix the toolbar")).toBeInTheDocument();
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
    expect(screen.getByRole("button", { name: "Open" })).toBeInTheDocument();
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
});
