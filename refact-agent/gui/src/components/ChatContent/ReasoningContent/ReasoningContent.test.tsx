import type React from "react";
import { fireEvent, render, screen } from "@testing-library/react";
import { act } from "react-dom/test-utils";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CollapsibleStoreProvider } from "../useStoredOpen";
import type { CollapsibleStore } from "../CollapsibleStore";
import { ReasoningContent } from "./ReasoningContent";

vi.mock("../../Markdown", () => ({
  Markdown: ({ children }: { children: React.ReactNode }) => (
    <div data-testid="reasoning-markdown">{children}</div>
  ),
}));

vi.mock("../../../features/Buddy/reportBuddyFrontendError", () => ({
  addBuddyCrashBreadcrumb: vi.fn(),
  setBuddyCrashHotSlot: vi.fn(),
}));

function renderReasoning(
  props: Partial<React.ComponentProps<typeof ReasoningContent>> = {},
  store?: CollapsibleStore,
) {
  const element = (
    <ReasoningContent
      reasoningContent="Reasoning details are visible"
      onCopyClick={vi.fn()}
      {...props}
    />
  );

  return render(
    store ? (
      <CollapsibleStoreProvider value={store}>{element}</CollapsibleStoreProvider>
    ) : (
      element
    ),
  );
}

describe("ReasoningContent", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("keeps actively streaming reasoning visible", () => {
    const storedOpen = new Map<string, boolean>([["reasoning:1", false]]);
    const store: CollapsibleStore = {
      get: (key) => storedOpen.get(key),
      set: (key, open) => storedOpen.set(key, open),
    };

    renderReasoning(
      {
        isStreaming: true,
        hasMessageContent: false,
        stateKey: "reasoning:1",
      },
      store,
    );

    expect(screen.getByText("Thinking...")).toBeInTheDocument();
    expect(screen.getByText("Reasoning details are visible")).toBeInTheDocument();
  });

  it("opens collapsed reasoning and reveals the body", () => {
    const storedOpen = new Map<string, boolean>([["reasoning:2", false]]);
    const store: CollapsibleStore = {
      get: (key) => storedOpen.get(key),
      set: (key, open) => storedOpen.set(key, open),
    };

    renderReasoning({ stateKey: "reasoning:2" }, store);

    expect(screen.getByText("Thought")).toBeInTheDocument();
    expect(
      screen.queryByText("Reasoning details are visible"),
    ).not.toBeInTheDocument();

    fireEvent.click(screen.getByText("Thought"));

    expect(screen.getByText("Reasoning details are visible")).toBeInTheDocument();
    expect(screen.getByText("Reasoning details are visible").closest(".is-open"))
      .toBeTruthy();
  });

  it("shows a header for historical reasoning blocks", () => {
    renderReasoning();

    expect(screen.getByText("Thought")).toBeInTheDocument();
  });

  it("reopens after auto-collapse and reveals content again", () => {
    vi.useFakeTimers();
    const { rerender } = renderReasoning({
      isStreaming: true,
      hasMessageContent: false,
    });

    expect(screen.getByText("Reasoning details are visible")).toBeInTheDocument();

    rerender(
      <ReasoningContent
        reasoningContent="Reasoning details are visible"
        onCopyClick={vi.fn()}
        isStreaming={false}
        hasMessageContent={true}
      />,
    );

    act(() => {
      vi.advanceTimersByTime(500);
    });
    act(() => {
      vi.advanceTimersByTime(150);
    });

    expect(screen.getByText(/Thought for/u)).toBeInTheDocument();
    expect(
      screen.queryByText("Reasoning details are visible"),
    ).not.toBeInTheDocument();

    fireEvent.click(screen.getByText(/Thought for/u));

    expect(screen.getByText("Reasoning details are visible")).toBeInTheDocument();
    expect(screen.getByText("Reasoning details are visible").closest(".is-open"))
      .toBeTruthy();
  });
});
