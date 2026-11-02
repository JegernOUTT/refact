import { useState } from "react";
import { afterEach, expect, test, vi } from "vitest";
import { render, screen, waitFor } from "../../utils/test-utils";
import { act } from "react-dom/test-utils";
import { ComposerToolbar } from "./ComposerToolbar";

function DraftControl() {
  const [draft, setDraft] = useState("");
  return (
    <input
      aria-label="Scope draft"
      value={draft}
      onChange={(event) => setDraft(event.target.value)}
    />
  );
}

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

test("measures individual controls, hides rare-first, restores reverse, and retains state", async () => {
  let width = 400;
  let measure: () => void = () => undefined;
  vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockImplementation(
    () => width,
  );
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(
    function (this: HTMLElement) {
      return {
        width: this.dataset.composerControl ? 80 : width,
        height: 28,
        x: 0,
        y: 0,
        left: 0,
        right: 80,
        top: 0,
        bottom: 28,
        toJSON: () => ({}),
      };
    },
  );
  vi.stubGlobal(
    "ResizeObserver",
    class {
      constructor(callback: () => void) {
        measure = callback;
      }
      observe() {
        return undefined;
      }
      disconnect() {
        return undefined;
      }
    },
  );
  const items = [
    {
      id: "model",
      row: "bottom" as const,
      priority: 2,
      content: <DraftControl />,
    },
    {
      id: "mode",
      row: "bottom" as const,
      priority: 3,
      content: <button>Mode</button>,
    },
    {
      id: "worktree",
      row: "bottom" as const,
      priority: 1,
      content: <button>Worktree</button>,
    },
    {
      id: "send",
      row: "bottom" as const,
      priority: 100,
      pinned: true,
      content: <button>Stop</button>,
    },
  ];
  const { user, rerender } = render(
    <ComposerToolbar
      items={items}
      editor={
        <button aria-controls="editor" data-composer-expansion="true">
          Expand
        </button>
      }
      menuCount={0}
    />,
  );
  const draft = screen.getByRole("textbox", { name: "Scope draft" });
  await user.type(draft, "keep scope");
  const host = (id: string) =>
    document.querySelector(`[data-composer-control="${id}"]`);
  width = 250;
  act(() => measure());
  await waitFor(() =>
    expect(host("worktree")).toHaveAttribute("data-hidden", "true"),
  );
  expect(host("model")).toHaveAttribute("data-hidden", "false");
  width = 170;
  act(() => measure());
  await waitFor(() =>
    expect(host("model")).toHaveAttribute("data-hidden", "true"),
  );
  expect(host("send")).toHaveAttribute("data-hidden", "false");
  width = 250;
  act(() => measure());
  await waitFor(() =>
    expect(host("model")).toHaveAttribute("data-hidden", "false"),
  );
  expect(host("worktree")).toHaveAttribute("data-hidden", "true");
  expect(draft).toHaveValue("keep scope");
  rerender(
    <ComposerToolbar
      items={items}
      editor={<button>Expand</button>}
      menuCount={1}
    />,
  );
  width = 400;
  act(() => measure());
  await waitFor(() =>
    expect(host("worktree")).toHaveAttribute("data-hidden", "false"),
  );
  rerender(
    <ComposerToolbar
      items={items}
      editor={<button>Expand</button>}
      menuCount={0}
    />,
  );
  await waitFor(() =>
    expect(host("worktree")).toHaveAttribute("data-hidden", "false"),
  );
  expect(screen.getByRole("textbox", { name: "Scope draft" })).toBe(draft);
  expect(
    screen.queryByRole("button", { name: /More/ }),
  ).not.toBeInTheDocument();
});
