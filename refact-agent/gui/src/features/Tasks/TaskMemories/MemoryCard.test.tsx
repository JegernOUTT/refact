import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it, vi } from "vitest";
import type React from "react";
import { render, screen } from "../../../utils/test-utils";
import { MemoryCard } from "./MemoryCard";
import type { TaskMemoryEntry } from "../../../services/refact/taskMemoriesApi";
import { memoryKindColor } from "../../../services/refact/taskKinds";

HTMLElement.prototype.hasPointerCapture = () => false;

const __dirname = dirname(fileURLToPath(import.meta.url));

const CONFIG_STATE = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "web" as const,
  },
};

const mockMemory: TaskMemoryEntry = {
  filename: "decision.md",
  created_at: "2026-05-22T01:00:00Z",
  created_at_known: true,
  title: "Use scoped memory index",
  content:
    "Keep memory search local to the current task. This preview has enough detail to invite expansion when future agents need the full context without making the inbox noisy by default. Extra words keep it long.",
  tags: ["planner", "search"],
  kind: "decision",
  namespace: "task",
  pinned: false,
  status: "active",
};

function renderCard(
  memory: TaskMemoryEntry,
  options: Partial<React.ComponentProps<typeof MemoryCard>> = {},
) {
  return render(
    <MemoryCard
      memory={memory}
      onPin={vi.fn()}
      onArchive={vi.fn()}
      {...options}
    />,
    { preloadedState: CONFIG_STATE },
  );
}

describe("MemoryCard", () => {
  it("renders title from frontmatter when present", () => {
    renderCard(mockMemory);

    expect(screen.getByText("Use scoped memory index")).toBeInTheDocument();
    expect(screen.queryByText("decision.md")).not.toBeInTheDocument();
  });

  it("falls back to the first content line when title is empty", () => {
    renderCard({
      ...mockMemory,
      title: "",
      content: "First useful content line\nSecond line",
    });

    expect(screen.getByText("First useful content line")).toBeInTheDocument();
    expect(screen.queryByText("decision.md")).not.toBeInTheDocument();
  });

  it("shows no title placeholder when title and content are empty", () => {
    renderCard({ ...mockMemory, title: "", content: "" });

    const title = screen.getByText("(no title)");
    expect(title).toBeInTheDocument();
    expect(title.className).toContain("cardTitleEmpty");
    expect(screen.queryByText("decision.md")).not.toBeInTheDocument();
  });

  it("memory card shows pin and archive icon buttons", () => {
    renderCard(mockMemory);

    expect(screen.getByRole("button", { name: "Pin" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Archive" })).toBeInTheDocument();
  });

  it("clicking the pin icon button toggles pinned", async () => {
    const onPin = vi.fn();
    const { user } = renderCard(mockMemory, { onPin });

    await user.click(screen.getByRole("button", { name: "Pin" }));
    expect(onPin).toHaveBeenCalledWith(mockMemory.filename, !mockMemory.pinned);
  });

  it("archive icon opens confirm popover and confirm archives", async () => {
    const onArchive = vi.fn();
    const { user } = renderCard(mockMemory, { onArchive });

    await user.click(screen.getByRole("button", { name: "Archive" }));
    expect(screen.getByText("Archive this memory?")).toBeInTheDocument();
    expect(onArchive).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Confirm archive" }));
    expect(onArchive).toHaveBeenCalledWith(mockMemory.filename);
  });

  it("clicking the row body toggles expansion", async () => {
    const { user } = renderCard(mockMemory);

    expect(
      screen.queryByTestId("memory-card-expanded-decision.md"),
    ).not.toBeInTheDocument();
    await user.click(
      screen.getByRole("button", {
        name: /Expand memory Use scoped memory index/i,
      }),
    );

    expect(
      screen.getByTestId("memory-card-expanded-decision.md"),
    ).toBeInTheDocument();
    expect(
      screen.getByTestId("memory-card-frontmatter-decision.md"),
    ).toBeInTheDocument();
    expect(screen.getByText("created_at")).toBeInTheDocument();
  });

  it("unknown_memory_kind_renders_gray_badge", () => {
    expect(memoryKindColor("sprint")).toBe("gray");
    expect(memoryKindColor("roadmap")).toBe("gray");
    expect(memoryKindColor("")).toBe("gray");
    expect(memoryKindColor("decision")).toBe("purple");
  });

  it("tag overflow uses shared thin scrollbar class", () => {
    const css = readFileSync(
      resolve(__dirname, "MemoryInboxPanel.module.css"),
      "utf-8",
    );
    expect(css).toMatch(/\.tagChips\s*\{[^}]*composes:[^}]*scrollbarThin/s);
    expect(css).toMatch(
      /\.expandedContent\s*\{[^}]*composes:[^}]*scrollbarThin/s,
    );
  });
});
