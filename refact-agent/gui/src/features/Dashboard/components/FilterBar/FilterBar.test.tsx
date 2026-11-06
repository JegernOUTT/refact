import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it, vi } from "vitest";
import { act } from "react-dom/test-utils";
import { fireEvent, render, screen } from "../../../../utils/test-utils";
import { FilterBar } from "./FilterBar";
import type { ChatHistoryItem } from "../../../History/historySlice";

const NOW = Date.now();

const css = readFileSync(
  join(
    process.cwd(),
    "src/features/Dashboard/components/FilterBar/FilterBar.module.css",
  ),
  "utf8",
);

function block(css: string, header: string): string {
  const start = css.indexOf(header);
  expect(start, `missing block: ${header}`).toBeGreaterThanOrEqual(0);
  let depth = 0;
  for (let i = start + header.length - 1; i < css.length; i++) {
    if (css[i] === "{") depth += 1;
    if (css[i] === "}") {
      depth -= 1;
      if (depth === 0) return css.slice(start, i + 1);
    }
  }
  throw new Error(`unterminated block: ${header}`);
}

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

describe("FilterBar narrow-width layout contract", () => {
  it("lets the row wrap instead of squeezing the search field off-screen", () => {
    expect(block(css, ".filterBar {")).toContain("flex-wrap: wrap;");
  });

  it("gives the segmented control and search field their own rows when narrow", () => {
    const narrow = block(css, "@container (max-width: 420px) {");

    expect(block(narrow, ".segmented {")).toContain("flex: 1 1 100%;");

    const searchField = block(narrow, ".searchField {");
    expect(searchField).toContain("flex-basis: 100%;");
    expect(searchField).toContain("min-width: 0;");
  });

  it("uses spacing tokens rather than magic numbers for the row gap", () => {
    expect(block(css, ".filterBar {")).toContain("gap: var(--rf-space-2);");
  });
});

describe("FilterBar controls", () => {
  it("renders only kind filters and search even when chats need attention", () => {
    render(<FilterBar filter={ALL_FILTER} onFilterChange={vi.fn()} />, {
      preloadedState: preloadedStateWith([
        makeChat({ id: "a" }),
        makeChat({ id: "b", session_state: "error" }),
      ]),
    });

    expect(screen.getByRole("radiogroup")).toBeInTheDocument();
    expect(screen.getAllByRole("radio")).toHaveLength(3);
    expect(screen.getByRole("radio", { name: "All" })).toBeChecked();
    expect(screen.getByRole("radio", { name: "Chats 2" })).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: "Tasks 0" })).toBeInTheDocument();
    expect(
      screen.getByRole("searchbox", { name: "Search chats and tasks" }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
    expect(screen.queryByText(/today/)).not.toBeInTheDocument();
  });

  it("changes kind while preserving the current search query", async () => {
    const onFilterChange = vi.fn();
    const { user } = render(
      <FilterBar
        filter={{ kind: "all", query: "existing" }}
        onFilterChange={onFilterChange}
      />,
    );

    await user.click(screen.getByRole("radio", { name: /^Chats/ }));
    expect(onFilterChange).toHaveBeenLastCalledWith({
      kind: "chat",
      query: "existing",
    });
    await user.click(screen.getByRole("radio", { name: /^Tasks/ }));
    expect(onFilterChange).toHaveBeenLastCalledWith({
      kind: "task",
      query: "existing",
    });
  });

  it("debounces search and uses the latest selected kind", () => {
    vi.useFakeTimers();
    try {
      const onFilterChange = vi.fn();
      const { rerender, unmount } = render(
        <FilterBar filter={ALL_FILTER} onFilterChange={onFilterChange} />,
      );
      const input = screen.getByRole("searchbox");
      fireEvent.change(input, { target: { value: "first" } });
      act(() => {
        vi.advanceTimersByTime(100);
      });
      fireEvent.change(input, { target: { value: "latest" } });
      rerender(
        <FilterBar
          filter={{ kind: "task", query: "" }}
          onFilterChange={onFilterChange}
        />,
      );
      act(() => {
        vi.advanceTimersByTime(199);
      });
      expect(onFilterChange).not.toHaveBeenCalled();
      act(() => {
        vi.advanceTimersByTime(1);
      });
      expect(onFilterChange).toHaveBeenCalledExactlyOnceWith({
        kind: "task",
        query: "latest",
      });
      fireEvent.change(input, { target: { value: "cancelled" } });
      unmount();
      act(() => {
        vi.advanceTimersByTime(200);
      });
      expect(onFilterChange).toHaveBeenCalledTimes(1);
    } finally {
      vi.useRealTimers();
    }
  });
});
