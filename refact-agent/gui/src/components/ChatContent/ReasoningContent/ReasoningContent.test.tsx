import type React from "react";
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { CollapsibleStoreProvider } from "../useStoredOpen";
import type { CollapsibleStore } from "../CollapsibleStore";
import { ReasoningContent } from "./ReasoningContent";

vi.mock("../../../features/Buddy/reportBuddyFrontendError", () => ({
  addBuddyCrashBreadcrumb: vi.fn(),
  setBuddyCrashHotSlot: vi.fn(),
}));

const CONTENT = "**Planning** the change\nchecking files\nlast line now";

function memoryStore(entries: [string, boolean][] = []): CollapsibleStore {
  const map = new Map(entries);
  return {
    get: (key) => map.get(key),
    set: (key, open) => map.set(key, open),
  };
}

function element(
  props: Partial<React.ComponentProps<typeof ReasoningContent>> = {},
) {
  return (
    <ReasoningContent
      reasoningContent={CONTENT}
      onCopyClick={vi.fn()}
      {...props}
    />
  );
}

function renderReasoning(
  props: Partial<React.ComponentProps<typeof ReasoningContent>> = {},
  store: CollapsibleStore = memoryStore(),
) {
  return render(
    <CollapsibleStoreProvider value={store}>
      {element(props)}
    </CollapsibleStoreProvider>,
  );
}

const toggle = () => screen.getByTestId("transcript-toggle");

describe("ReasoningContent", () => {
  it("renders the Thought row with the first line as target", () => {
    renderReasoning();
    expect(screen.getByTestId("transcript-verb").textContent).toBe("Thought");
    expect(screen.getByTestId("transcript-target").textContent).toBe(
      "Planning the change",
    );
    expect(toggle()).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByText("checking files")).toBeNull();
  });

  it("stays collapsed while streaming and shows a live last-line hint", () => {
    const { rerender } = renderReasoning({
      isStreaming: true,
      hasMessageContent: false,
      stateKey: "re:1",
    });
    expect(toggle()).toHaveAttribute("aria-expanded", "false");
    expect(
      screen.getByTestId("reasoning-content").getAttribute("data-status"),
    ).toBe("running");
    expect(screen.getByTestId("transcript-meta").textContent).toBe(
      "last line now",
    );
    rerender(
      <CollapsibleStoreProvider value={memoryStore()}>
        {element({ isStreaming: false, hasMessageContent: true })}
      </CollapsibleStoreProvider>,
    );
    expect(toggle()).toHaveAttribute("aria-expanded", "false");
  });

  it("opens only when the user asks and keeps it open across updates", () => {
    const store = memoryStore();
    const { rerender } = renderReasoning(
      { isStreaming: true, stateKey: "re:2" },
      store,
    );
    fireEvent.click(toggle());
    expect(toggle()).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText(/checking files/)).toBeInTheDocument();
    rerender(
      <CollapsibleStoreProvider value={store}>
        {element({
          isStreaming: false,
          hasMessageContent: true,
          stateKey: "re:2",
        })}
      </CollapsibleStoreProvider>,
    );
    expect(toggle()).toHaveAttribute("aria-expanded", "true");
    expect(store.get("re:2")).toBe(true);
  });

  it("restores the stored open state", () => {
    renderReasoning({ stateKey: "re:3" }, memoryStore([["re:3", true]]));
    expect(toggle()).toHaveAttribute("aria-expanded", "true");
  });
});
