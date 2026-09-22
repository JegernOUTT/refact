import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it, vi } from "vitest";
import { render, screen } from "../../../../utils/test-utils";
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

  it("right-aligns the attention chip and New button on the trailing row", () => {
    const narrow = block(css, "@container (max-width: 420px) {");

    expect(block(narrow, ".attentionChip {")).toContain("margin-left: auto;");
    expect(block(narrow, ".newButton {")).toContain("margin-left: auto;");
    expect(block(narrow, ".attentionChip ~ .newButton {")).toContain(
      "margin-left: 0;",
    );
  });

  it("orders the chip and New button after the search field when narrow", () => {
    const narrow = block(css, "@container (max-width: 420px) {");

    expect(block(narrow, ".attentionChip {")).toContain("order: 1;");
    expect(block(narrow, ".newButton {")).toContain("order: 1;");
  });

  it("keeps hiding the aggregate in the existing container query", () => {
    expect(block(css, "@container (max-width: 700px) {")).toContain(
      "display: none;",
    );
  });

  it("uses spacing tokens rather than magic numbers for the row gap", () => {
    expect(block(css, ".filterBar {")).toContain("gap: var(--rf-space-2);");
  });
});

describe("FilterBar controls", () => {
  it("renders the segmented control, search field and New button", () => {
    render(
      <FilterBar
        filter={ALL_FILTER}
        attentionActive={false}
        onFilterChange={vi.fn()}
        onToggleAttention={vi.fn()}
      />,
      { preloadedState: preloadedStateWith([makeChat({ id: "a" })]) },
    );

    expect(screen.getByRole("radiogroup")).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: "All" })).toBeInTheDocument();
    expect(screen.getByLabelText("Search chats and tasks")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "New" })).toBeInTheDocument();
  });

  it("also renders the attention chip when items need attention", () => {
    render(
      <FilterBar
        filter={ALL_FILTER}
        attentionActive={false}
        onFilterChange={vi.fn()}
        onToggleAttention={vi.fn()}
      />,
      {
        preloadedState: preloadedStateWith([
          makeChat({ id: "a" }),
          makeChat({
            id: "b",
            session_state: "error",
          } as Partial<ChatHistoryItem> & { id: string }),
        ]),
      },
    );

    expect(screen.getByRole("button", { pressed: false })).toBeInTheDocument();
    expect(screen.getByRole("radiogroup")).toBeInTheDocument();
    expect(screen.getByLabelText("Search chats and tasks")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "New" })).toBeInTheDocument();
  });
});
