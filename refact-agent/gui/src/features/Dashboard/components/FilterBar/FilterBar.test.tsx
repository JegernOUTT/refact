import { QueryStatus } from "@reduxjs/toolkit/query";
import { organizationApi } from "../../../../services/refact/organization";
import { http, HttpResponse } from "msw";
import { server } from "../../../../utils/mockServer";
import { organizationTestHandlers } from "../../organizationTestHandlers";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { act } from "react-dom/test-utils";
import {
  fireEvent,
  render,
  screen,
  waitFor,
} from "../../../../utils/test-utils";
import { FilterBar } from "./FilterBar";
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

describe("FilterBar controls", () => {
  it("renders only kind filters and search even when chats need attention", () => {
    render(<FilterBar filter={ALL_FILTER} onFilterChange={vi.fn()} />, {
      preloadedState: preloadedStateWith([
        makeChat({ id: "a" }),
        makeChat({ id: "b", session_state: "error" }),
      ]),
    });

    expect(screen.getByRole("combobox", { name: "Type" })).toBeInTheDocument();
    expect(
      screen.queryByRole("combobox", { name: "Topics" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Tags" }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByRole("searchbox", { name: "Search chats and tasks" }),
    ).toBeInTheDocument();

    expect(screen.queryByText(/today/)).not.toBeInTheDocument();
  });

  it("automatically reveals only available taxonomy controls after empty and error responses", async () => {
    let mode = "empty";
    server.use(
      http.get("*/v1/organization/taxonomy", ({ request }) => {
        const kind = new URL(request.url).searchParams.get("kind");
        if (mode === "error" || (mode === "partial" && kind === "tags"))
          return new HttpResponse(null, { status: 503 });
        return HttpResponse.json({
          revision: 1,
          items:
            mode === "empty"
              ? []
              : [
                  {
                    id: kind,
                    name: kind === "topics" ? "Interface" : "Review",
                    archived: false,
                  },
                ],
          next_cursor: null,
        });
      }),
    );
    const { store } = render(
      <FilterBar
        filter={{ ...ALL_FILTER, query: "existing" }}
        onFilterChange={vi.fn()}
      />,
    );
    await waitFor(() =>
      expect(
        Object.values(store.getState().organizationApi.queries).some(
          (query) => query?.status === QueryStatus.fulfilled,
        ),
      ).toBe(true),
    );
    expect(
      screen.queryByRole("combobox", { name: "Topics" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Tags" }),
    ).not.toBeInTheDocument();
    mode = "error";
    act(() => {
      store.dispatch(organizationApi.util.invalidateTags(["Organization"]));
    });
    await waitFor(() =>
      expect(
        Object.values(store.getState().organizationApi.queries).some(
          (query) => query?.status === QueryStatus.rejected,
        ),
      ).toBe(true),
    );
    expect(
      screen.queryByRole("combobox", { name: "Topics" }),
    ).not.toBeInTheDocument();
    mode = "partial";
    act(() => {
      store.dispatch(organizationApi.util.invalidateTags(["Organization"]));
    });
    await waitFor(() =>
      expect(
        screen.getByRole("combobox", { name: "Topics" }),
      ).toBeInTheDocument(),
    );
    expect(
      screen.queryByRole("button", { name: "Tags" }),
    ).not.toBeInTheDocument();
    mode = "ready";
    act(() => {
      store.dispatch(organizationApi.util.invalidateTags(["Organization"]));
    });
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Tags" })).toBeInTheDocument(),
    );
    expect(screen.getByRole("searchbox")).toHaveValue("existing");
    expect(screen.getByRole("combobox", { name: "Type" })).toBeEnabled();
  });

  it("changes kind while preserving the current search query", async () => {
    const onFilterChange = vi.fn();
    const { user } = render(
      <FilterBar
        filter={{ kind: "all", query: "existing" }}
        onFilterChange={onFilterChange}
      />,
    );

    await user.click(screen.getByRole("combobox", { name: "Type" }));
    await user.click(screen.getByRole("option", { name: /^Chats/ }));
    expect(onFilterChange).toHaveBeenLastCalledWith({
      kind: "chat",
      query: "existing",
    });
    await user.click(screen.getByRole("combobox", { name: "Type" }));
    await user.click(screen.getByRole("option", { name: /^Tasks/ }));
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

beforeEach(() => server.use(...organizationTestHandlers));

beforeEach(() => {
  Element.prototype.hasPointerCapture = () => false;
  Element.prototype.setPointerCapture = () => undefined;
  Element.prototype.releasePointerCapture = () => undefined;
  Element.prototype.scrollIntoView = () => undefined;
});
