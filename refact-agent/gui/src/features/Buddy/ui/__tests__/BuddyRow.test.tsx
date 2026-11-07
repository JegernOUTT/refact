import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { FileText } from "lucide-react";
import { BuddyRow } from "../BuddyRow";
import { BuddyStatusBadge } from "../BuddyStatusBadge";
import { BuddyTimestamp } from "../BuddyTimestamp";
import { BuddyErrorCallout, BuddyLoading } from "../BuddyStates";

const NOW = new Date(2026, 4, 15, 12, 0).getTime();

describe("BuddyRow", () => {
  it("renders a button row inside ul/li with chevron, meta and status", () => {
    const onClick = vi.fn();
    render(
      <ul>
        <li>
          <BuddyRow
            title="Fix flaky tests"
            leading={{ tick: "warning" }}
            status={<BuddyStatusBadge dimension="work" value="needs_you" />}
            meta={[
              "Error detective",
              null,
              <BuddyTimestamp
                key="t"
                value={new Date(NOW - 5 * 60_000).toISOString()}
                now={NOW}
              />,
            ]}
            onClick={onClick}
            data-desk-row="w1"
            testId="row"
          />
        </li>
      </ul>,
    );
    expect(document.querySelector("ul li button")).not.toBeNull();
    const button = screen.getByRole("button", { name: /Fix flaky tests/ });
    expect(button).toHaveAttribute("data-desk-row", "w1");
    expect(button).toHaveTextContent("Fix flaky tests");
    expect(button).toHaveTextContent("Needs you");
    expect(button).toHaveTextContent("Error detective · 5m ago");
    expect(screen.getByTitle("Fix flaky tests")).toBeInTheDocument();
    fireEvent.click(button);
    expect(onClick).toHaveBeenCalledTimes(1);
    expect(button).toHaveClass("rf-focus-ring-inset");
  });

  it("marks selection with aria-current or aria-selected", () => {
    const { rerender } = render(
      <BuddyRow title="Selected" selected onClick={() => undefined} />,
    );
    const row = screen.getByRole("button", { name: /Selected/ });
    expect(row).toHaveAttribute("aria-current", "true");
    expect(row.parentElement).toHaveAttribute("data-selected", "true");
    rerender(
      <BuddyRow
        title="Selected"
        selected={false}
        selection="selected"
        onClick={() => undefined}
      />,
    );
    expect(screen.getByRole("button", { name: /Selected/ })).toHaveAttribute(
      "aria-selected",
      "false",
    );
  });

  it("shows unread with a dot, weight hook and accessible text", () => {
    render(<BuddyRow title="New report" unread testId="unread" />);
    const row = screen.getByTestId("unread");
    expect(row).toHaveAttribute("data-unread", "true");
    expect(row).toHaveTextContent("(unread)");
  });

  it("disables interaction", async () => {
    const onClick = vi.fn();
    render(<BuddyRow title="Locked" disabled onClick={onClick} />);
    const row = screen.getByRole("button", { name: /Locked/ });
    expect(row).toBeDisabled();
    await userEvent.click(row);
    expect(onClick).not.toHaveBeenCalled();
  });

  it("renders actions instead of a chevron, outside the main button", () => {
    render(
      <BuddyRow
        title="With actions"
        leading={{ icon: FileText }}
        onClick={() => undefined}
        actions={<button type="button">Dismiss</button>}
        testId="actions"
      />,
    );
    const container = screen.getByTestId("actions");
    const main = within(container).getByRole("button", {
      name: /With actions/,
    });
    const action = within(container).getByRole("button", { name: "Dismiss" });
    expect(main.contains(action)).toBe(false);
    expect(main.querySelectorAll("svg")).toHaveLength(1);
  });

  it("renders static and link rows", () => {
    render(
      <>
        <BuddyRow title="Static" testId="static" />
        <BuddyRow title="Link" href="#report" />
      </>,
    );
    expect(screen.getByTestId("static").querySelector("button")).toBeNull();
    expect(screen.getByTestId("static").querySelector("svg")).toBeNull();
    expect(screen.getByRole("link", { name: /Link/ })).toHaveAttribute(
      "href",
      "#report",
    );
  });
});

describe("BuddyTimestamp", () => {
  it("renders a focusable time with absolute datetime", () => {
    const value = new Date(NOW - 2 * 3_600_000).toISOString();
    render(<BuddyTimestamp value={value} now={NOW} prefix="Updated" />);
    const time = screen.getByText("Updated 2h ago");
    expect(time.tagName).toBe("TIME");
    expect(time).toHaveAttribute("dateTime", value);
    expect(time).toHaveAttribute("tabindex", "0");
    expect(time.getAttribute("aria-label")).toContain("May 15, 2026");
  });

  it("renders the fallback for invalid values", () => {
    render(<BuddyTimestamp value="garbage" fallback="Unknown time" />);
    expect(screen.getByText("Unknown time")).toBeInTheDocument();
  });
});

describe("Buddy states", () => {
  it("renders an error with retry and closed technical details", async () => {
    const onRetry = vi.fn();
    render(
      <BuddyErrorCallout
        thing="reports"
        error={new Error("502 Bad Gateway")}
        onRetry={onRetry}
      />,
    );
    const alert = screen.getByRole("alert");
    expect(alert).toHaveTextContent("Couldn't load reports.");
    await userEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(onRetry).toHaveBeenCalledTimes(1);
    const details = screen.getByRole("button", { name: "Technical details" });
    expect(details).toHaveAttribute("aria-expanded", "false");
    expect(screen.getByText("502 Bad Gateway")).not.toBeVisible();
    await userEvent.click(details);
    expect(screen.getByText("502 Bad Gateway")).toBeVisible();
  });

  it("renders loading skeletons with a status role", () => {
    render(<BuddyLoading count={2} variant="cards" label="Loading reports" />);
    const status = screen.getByRole("status");
    expect(status).toHaveTextContent("Loading reports");
    expect(status.children).toHaveLength(3);
  });
});
