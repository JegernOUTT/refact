import { afterEach, describe, expect, it } from "vitest";
import { render, screen, within } from "../../../utils/test-utils";
import {
  BuddyRecentWarnings,
  type BuddyWarningRow,
} from "../BuddyRecentErrorsPanel";
import { readBuddyPersistentState, resetBuddyPersistentState } from "../ui";

const CONFIG_STATE = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "vscode" as const,
  },
};

function row(
  id: string,
  title: string,
  detail: string | null,
  count = 1,
): BuddyWarningRow {
  return {
    event: {
      id,
      signal_type: "chat_error",
      title,
      source: "test",
      status: "failed",
      priority: "high",
      created_at: new Date().toISOString(),
    },
    detail,
    count,
  };
}

afterEach(() => resetBuddyPersistentState());

describe("BuddyRecentWarnings", () => {
  it("humanizes codes, keeps one meta line per row and hides raw text in closed details", async () => {
    const rows = [
      row("a", "browser_error: Failed to start chrome", null),
      row("b", "Llm Error", "The provider returned 500. Retry later.", 3),
      row("c", "Generic", "Something went sideways."),
    ];
    const { user } = render(
      <BuddyRecentWarnings rows={rows} persistKey="test.warnings" />,
      { preloadedState: CONFIG_STATE },
    );
    const list = screen.getByRole("list", { name: "Recent warnings" });
    expect(
      within(list).getByText("Browser error: Failed to start chrome"),
    ).toBeInTheDocument();
    expect(
      within(list).getByText("Model error: The provider returned 500."),
    ).toBeInTheDocument();
    expect(
      within(list).getByText("Other: Something went sideways."),
    ).toBeInTheDocument();
    expect(within(list).queryByText(/browser_error/)).toBeNull();

    const items = within(list).getAllByRole("listitem");
    const llm = items[1];
    expect(within(llm).getByText("×3")).toBeInTheDocument();
    expect(llm.querySelector("time")).not.toBeNull();

    const details = within(llm).getByRole("button", {
      name: "Technical details",
    });
    expect(details).toHaveAttribute("aria-expanded", "false");
    expect(details).toHaveAttribute("aria-controls");
    await user.click(details);
    expect(within(llm).getByText(/Llm Error/)).toBeInTheDocument();
  });

  it("persists open details and pagination across remounts", async () => {
    const rows = Array.from({ length: 25 }, (_, index) =>
      row(`w${index}`, `Warning ${index}`, `Detail ${index}`),
    );
    const first = render(
      <BuddyRecentWarnings rows={rows} persistKey="test.persist" />,
      { preloadedState: CONFIG_STATE },
    );
    expect(screen.getAllByRole("listitem")).toHaveLength(20);
    await first.user.click(screen.getByRole("button", { name: "Show more" }));
    expect(screen.getAllByRole("listitem")).toHaveLength(25);
    const item = screen.getByText("Warning 0").closest("li");
    if (!item) throw new Error("missing row");
    await first.user.click(
      within(item).getByRole("button", { name: "Technical details" }),
    );
    first.unmount();

    render(<BuddyRecentWarnings rows={rows} persistKey="test.persist" />, {
      preloadedState: CONFIG_STATE,
    });
    expect(screen.getAllByRole("listitem")).toHaveLength(25);
    const again = screen.getByText("Warning 0").closest("li");
    if (!again) throw new Error("missing row");
    expect(
      within(again).getByRole("button", { name: "Technical details" }),
    ).toHaveAttribute("aria-expanded", "true");
  });

  it("prunes persisted open warning ids to the current rows", async () => {
    const first = render(
      <BuddyRecentWarnings
        rows={[row("old", "Old warning", "Detail")]}
        persistKey="test.bounded"
      />,
      { preloadedState: CONFIG_STATE },
    );
    await first.user.click(
      screen.getByRole("button", { name: "Technical details" }),
    );
    expect(readBuddyPersistentState("test.bounded.openIds")).toEqual(["old"]);
    first.rerender(
      <BuddyRecentWarnings
        rows={[row("new", "New warning", "Detail")]}
        persistKey="test.bounded"
      />,
    );
    expect(readBuddyPersistentState("test.bounded.openIds")).toEqual([]);
    expect(
      readBuddyPersistentState("test.bounded.details.old"),
    ).toBeUndefined();
  });

  it("shows a compact empty state", () => {
    render(<BuddyRecentWarnings rows={[]} persistKey="test.empty" />, {
      preloadedState: CONFIG_STATE,
    });
    expect(
      screen.getByText("No warnings in the last six hours"),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("heading", { name: "Recent warnings" }),
    ).toBeInTheDocument();
  });
});

describe("BuddyRecentWarnings workflow titles", () => {
  it("names workflows instead of showing their ids", () => {
    render(
      <BuddyRecentWarnings
        rows={[
          row("wf", "refact_error_detective failed: Workflow failed", null),
          row("net", "Network", "Connection reset by peer. Retrying."),
        ]}
        persistKey="test.workflow"
      />,
      { preloadedState: CONFIG_STATE },
    );
    expect(
      screen.getByText("Error detective failed: Workflow failed"),
    ).toBeInTheDocument();
    expect(
      screen.getByText("Network error: Connection reset by peer."),
    ).toBeInTheDocument();
    expect(screen.queryByText(/refact_error_detective/)).toBeNull();
  });
});

describe("BuddyRecentWarnings bracketed codes", () => {
  it("keeps bracketed engine codes out of the title", () => {
    render(
      <BuddyRecentWarnings
        rows={[
          row(
            "crash",
            "Generic",
            "[frontend:possible_renderer_crash] Possible renderer crash detected.",
          ),
        ]}
        persistKey="test.bracket"
      />,
      { preloadedState: CONFIG_STATE },
    );
    expect(
      screen.getByText("Other: Possible renderer crash detected."),
    ).toBeInTheDocument();
    expect(screen.queryByText(/possible_renderer_crash/)).toBeNull();
  });
});
