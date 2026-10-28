import React from "react";
import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { FileText } from "lucide-react";
import { TranscriptItem } from "./TranscriptItem";
import { TranscriptClamp, TranscriptRaw } from "./TranscriptParts";
import { CollapsibleStoreProvider } from "../useStoredOpen";
import type { CollapsibleStore } from "../CollapsibleStore";

const icon = <FileText aria-hidden />;

function setReducedMotion(reduce: boolean) {
  vi.spyOn(window, "matchMedia").mockImplementation(
    (query: string) =>
      ({
        matches: reduce && query.includes("reduce"),
        media: query,
        onchange: null,
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
        addListener: vi.fn(),
        removeListener: vi.fn(),
        dispatchEvent: vi.fn(),
      }) as MediaQueryList,
  );
}

describe("TranscriptItem", () => {
  it("renders the row anatomy", () => {
    render(
      <TranscriptItem
        icon={icon}
        meta="42 lines"
        output="body"
        quickAction={<button type="button">Stop</button>}
        status="running"
        target="src/main.rs"
        verb="Read"
      />,
    );
    const row = screen.getByTestId("transcript-row");
    expect(within(row).getByText("Read")).toBeInTheDocument();
    expect(within(row).getByText("·")).toBeInTheDocument();
    expect(within(row).getByText("src/main.rs")).toBeInTheDocument();
    expect(screen.getByTestId("transcript-meta")).toHaveTextContent("42 lines");
    expect(screen.getByTestId("transcript-status-pill")).toHaveTextContent(
      "Running",
    );
    expect(row.lastElementChild).toBe(screen.getByTestId("transcript-chevron"));
  });

  it("uses an arrow separator for messages", () => {
    render(
      <TranscriptItem separator="arrow" target="worker-1" verb="Message" />,
    );
    expect(screen.getByText("→")).toBeInTheDocument();
  });

  it("hides the pill for done unless statusLabel is set", () => {
    const { rerender } = render(<TranscriptItem status="done" verb="Read" />);
    expect(screen.queryByTestId("transcript-status-pill")).toBeNull();
    rerender(
      <TranscriptItem status="done" statusLabel="2 console errors" verb="R" />,
    );
    expect(screen.getByTestId("transcript-status-pill")).toHaveTextContent(
      "2 console errors",
    );
  });

  it("renders sections in the fixed order and supports renaming", () => {
    render(
      <TranscriptItem
        actions={<button type="button">Retry</button>}
        artifacts="a.png"
        defaultOpen
        error="boom"
        input="args"
        output="out"
        raw={[{ label: "Request", value: { a: 1 } }]}
        sectionLabels={{ output: "Console" }}
        summary={[{ label: "URL", value: "x" }]}
        verb="Browse"
      />,
    );
    const order = Array.from(
      screen
        .getByTestId("transcript-panel")
        .querySelectorAll<HTMLElement>("[data-section]"),
    ).map((node) => node.dataset.section);
    expect(order).toEqual([
      "summary",
      "input",
      "output",
      "artifacts",
      "error",
      "actions",
      "raw",
    ]);
    expect(screen.getByText("Console")).toBeInTheDocument();
  });

  it("toggles with Enter and Space and keeps aria in sync", async () => {
    const user = userEvent.setup();
    setReducedMotion(true);
    render(<TranscriptItem output="out" verb="Read" />);
    const row = screen.getByRole("button", { name: /Read/ });
    expect(row).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByTestId("transcript-panel")).toBeNull();
    row.focus();
    await user.keyboard("{Enter}");
    expect(row).toHaveAttribute("aria-expanded", "true");
    const panel = screen.getByTestId("transcript-panel");
    expect(row.getAttribute("aria-controls")).toBe(panel.id);
    await user.keyboard(" ");
    expect(row).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByTestId("transcript-panel")).toBeNull();
  });

  it("shows no chevron and no button when nothing can expand", () => {
    render(<TranscriptItem target="note" verb="Notice" />);
    expect(screen.queryByTestId("transcript-chevron")).toBeNull();
    expect(screen.queryByRole("button")).toBeNull();
    expect(screen.queryByTestId("transcript-toggle")).toBeNull();
  });

  it("does not toggle when the quick action is clicked", () => {
    render(
      <TranscriptItem
        output="out"
        quickAction={<button type="button">Stop</button>}
        verb="Run"
      />,
    );
    fireEvent.click(screen.getByText("Stop"));
    expect(screen.getByTestId("transcript-toggle")).toHaveAttribute(
      "aria-expanded",
      "false",
    );
  });

  it("collapses instantly under reduced motion", () => {
    setReducedMotion(true);
    render(<TranscriptItem defaultOpen output="out" verb="Read" />);
    expect(screen.getByTestId("transcript-panel")).toHaveAttribute(
      "data-expanded",
      "true",
    );
    fireEvent.click(screen.getByTestId("transcript-row"));
    expect(screen.queryByTestId("transcript-panel")).toBeNull();
  });

  it("keeps the panel mounted while the collapse animates", () => {
    setReducedMotion(false);
    render(<TranscriptItem defaultOpen output="out" verb="Read" />);
    fireEvent.click(screen.getByTestId("transcript-row"));
    const panel = screen.getByTestId("transcript-panel");
    expect(panel).toHaveAttribute("data-expanded", "false");
    fireEvent.transitionEnd(panel);
    expect(screen.queryByTestId("transcript-panel")).toBeNull();
  });

  it("persists open state across remounts via openKey", () => {
    const map = new Map<string, boolean>();
    const store: CollapsibleStore = {
      get: (key) => map.get(key),
      set: (key, value) => void map.set(key, value),
    };
    const ui = (
      <CollapsibleStoreProvider value={store}>
        <TranscriptItem openKey="item-1" output="out" verb="Read" />
      </CollapsibleStoreProvider>
    );
    const { unmount } = render(ui);
    fireEvent.click(screen.getByTestId("transcript-row"));
    unmount();
    render(ui);
    expect(screen.getByTestId("transcript-toggle")).toHaveAttribute(
      "aria-expanded",
      "true",
    );
  });
});

describe("TranscriptClamp", () => {
  it("toggles Show all and Show less when content overflows", () => {
    const scrollHeight = vi
      .spyOn(HTMLElement.prototype, "scrollHeight", "get")
      .mockReturnValue(2000);
    const clientHeight = vi
      .spyOn(HTMLElement.prototype, "clientHeight", "get")
      .mockReturnValue(480);
    render(
      <TranscriptClamp lineCount={120}>
        <pre>long</pre>
      </TranscriptClamp>,
    );
    const button = screen.getByRole("button", { name: "Show all · 120 lines" });
    expect(screen.getByTestId("transcript-clamp")).toHaveAttribute(
      "data-clamped",
      "true",
    );
    fireEvent.click(button);
    expect(screen.getByRole("button", { name: "Show less" })).toBeVisible();
    expect(screen.getByTestId("transcript-clamp")).toHaveAttribute(
      "data-clamped",
      "false",
    );
    scrollHeight.mockRestore();
    clientHeight.mockRestore();
  });

  it("shows no toggle when content fits", () => {
    render(<TranscriptClamp>short</TranscriptClamp>);
    expect(screen.queryByRole("button")).toBeNull();
  });
});

describe("TranscriptRaw", () => {
  it("is collapsed by default and pretty prints blocks", () => {
    setReducedMotion(true);
    render(<TranscriptRaw blocks={[{ label: "Args", value: '{"a":1}' }]} />);
    const toggle = screen.getByRole("button", { name: "Raw" });
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByText(/"a": 1/)).toBeNull();
    fireEvent.click(toggle);
    expect(screen.getByText(/"a": 1/)).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Copy Args" }),
    ).toBeInTheDocument();
  });
});

describe("TranscriptItem layout guard", () => {
  it("renders inline markup as target without changing roles", () => {
    render(<TranscriptItem target={<code>ls -la</code>} verb="Run" />);
    const title = screen.getByTestId("transcript-title");
    expect(within(title).getByText("ls -la").tagName).toBe("CODE");
  });
});

void React;
