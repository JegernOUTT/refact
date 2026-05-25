import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it, vi } from "vitest";
import { render, screen } from "../../../utils/test-utils";
import { MemoryCard } from "./MemoryCard";
import type { TaskMemoryEntry } from "../../../services/refact/taskMemoriesApi";

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

describe("MemoryCard", () => {
  it("memory card shows pin and archive icon buttons", () => {
    render(
      <MemoryCard memory={mockMemory} onPin={vi.fn()} onArchive={vi.fn()} />,
      { preloadedState: CONFIG_STATE },
    );

    expect(screen.getByRole("button", { name: "Pin" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Archive" })).toBeInTheDocument();
  });

  it("clicking the pin icon button toggles pinned", async () => {
    const onPin = vi.fn();
    const { user } = render(
      <MemoryCard memory={mockMemory} onPin={onPin} onArchive={vi.fn()} />,
      { preloadedState: CONFIG_STATE },
    );

    await user.click(screen.getByRole("button", { name: "Pin" }));
    expect(onPin).toHaveBeenCalledWith(mockMemory.filename, !mockMemory.pinned);
  });

  it("archive icon opens confirm popover and confirm archives", async () => {
    const onArchive = vi.fn();
    const { user } = render(
      <MemoryCard memory={mockMemory} onPin={vi.fn()} onArchive={onArchive} />,
      { preloadedState: CONFIG_STATE },
    );

    await user.click(screen.getByRole("button", { name: "Archive" }));
    expect(screen.getByText("Archive this memory?")).toBeInTheDocument();
    expect(onArchive).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Confirm archive" }));
    expect(onArchive).toHaveBeenCalledWith(mockMemory.filename);
  });

  it("tag overflow uses shared thin scrollbar class", () => {
    const css = readFileSync(
      resolve(__dirname, "MemoryInboxPanel.module.css"),
      "utf-8",
    );
    expect(css).toMatch(/\.tagChips\s*\{[^}]*composes:[^}]*scrollbarThin/s);
  });
});
