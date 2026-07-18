import { fireEvent, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { render } from "../../utils/test-utils";
import { LogViewer, type LogViewerProps } from "./LogViewer";
import type { BugReportSource, LogLine } from "./useBugReportSources";

function lines(count: number): LogLine[] {
  return Array.from({ length: count }, (_, index) => ({
    text: `12:00:${String(index).padStart(2, "0")} INFO line ${index + 1}`,
    level: "info",
  }));
}

function sourceWithLines(count: number): BugReportSource {
  return {
    key: "daemon",
    label: "Daemon",
    available: true,
    exists: true,
    path: "/tmp/daemon.log",
    lines: lines(count),
    errorCount: 0,
  };
}

function defaultProps(source: BugReportSource): LogViewerProps {
  return {
    source,
    filter: "",
    levelFilter: "all",
    paused: false,
    onFilterChange: vi.fn(),
    onLevelFilterChange: vi.fn(),
    onTogglePaused: vi.fn(),
  };
}

function mockGeometry(element: HTMLElement) {
  let scrollTop = 0;

  Object.defineProperty(element, "scrollHeight", {
    configurable: true,
    get: () => 1000,
  });
  Object.defineProperty(element, "clientHeight", {
    configurable: true,
    get: () => 200,
  });
  Object.defineProperty(element, "scrollTop", {
    configurable: true,
    get: () => scrollTop,
    set: (value: number) => {
      scrollTop = value;
    },
  });
}

describe("LogViewer", () => {
  it("shows a new-lines pill after user scrolls away from bottom and lines arrive", () => {
    const { rerender } = render(
      <LogViewer {...defaultProps(sourceWithLines(2))} />,
    );
    const view = screen.getByTestId("bug-report-log-view");
    mockGeometry(view);

    view.scrollTop = 100;
    fireEvent.scroll(view);

    rerender(<LogViewer {...defaultProps(sourceWithLines(5))} />);

    expect(
      screen.getByRole("button", { name: "↓ 3 new lines" }),
    ).toBeInTheDocument();
  });

  it("clicking the new-lines pill resumes follow mode and hides the pill", async () => {
    const { rerender, user } = render(
      <LogViewer {...defaultProps(sourceWithLines(2))} />,
    );
    const view = screen.getByTestId("bug-report-log-view");
    mockGeometry(view);

    view.scrollTop = 100;
    fireEvent.scroll(view);
    rerender(<LogViewer {...defaultProps(sourceWithLines(4))} />);

    await user.click(screen.getByRole("button", { name: "↓ 2 new lines" }));

    expect(
      screen.queryByRole("button", { name: /new lines|latest/ }),
    ).not.toBeInTheDocument();
  });

  it("re-enables follow mode when the user scrolls back to the bottom", () => {
    render(<LogViewer {...defaultProps(sourceWithLines(2))} />);
    const view = screen.getByTestId("bug-report-log-view");
    mockGeometry(view);
    const followButton = screen.getByRole("button", { name: "Follow tail" });

    view.scrollTop = 100;
    fireEvent.scroll(view);
    expect(followButton).toHaveAttribute("aria-pressed", "false");

    view.scrollTop = 800;
    fireEvent.scroll(view);
    expect(followButton).toHaveAttribute("aria-pressed", "true");
  });
  it("keeps the pill count stable when filters change while detached", () => {
    const { rerender } = render(
      <LogViewer {...defaultProps(sourceWithLines(2))} />,
    );
    const view = screen.getByTestId("bug-report-log-view");
    mockGeometry(view);

    view.scrollTop = 100;
    fireEvent.scroll(view);
    rerender(<LogViewer {...defaultProps(sourceWithLines(4))} />);
    expect(
      screen.getByRole("button", { name: "↓ 2 new lines" }),
    ).toBeInTheDocument();

    rerender(
      <LogViewer {...defaultProps(sourceWithLines(4))} filter="nomatch" />,
    );

    expect(
      screen.getByRole("button", { name: "↓ 2 new lines" }),
    ).toBeInTheDocument();
  });

  it("resets follow and pill when the source changes", () => {
    const { rerender } = render(
      <LogViewer {...defaultProps(sourceWithLines(2))} />,
    );
    const view = screen.getByTestId("bug-report-log-view");
    mockGeometry(view);

    view.scrollTop = 100;
    fireEvent.scroll(view);
    rerender(<LogViewer {...defaultProps(sourceWithLines(4))} />);
    expect(
      screen.getByRole("button", { name: "↓ 2 new lines" }),
    ).toBeInTheDocument();

    rerender(
      <LogViewer
        {...defaultProps({
          ...sourceWithLines(3),
          key: "engine",
          label: "Engine",
        })}
      />,
    );

    expect(
      screen.queryByRole("button", { name: /new lines|latest/ }),
    ).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Follow tail" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
  });
});
