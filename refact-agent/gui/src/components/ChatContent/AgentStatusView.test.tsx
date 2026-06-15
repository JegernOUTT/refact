import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "../../utils/test-utils";
import {
  filterAgentStatusRows,
  formatAgentActionCommand,
  parseAgentStatusOutput,
  type AgentStatusReport,
} from "./AgentStatusModel";
import { AgentStatusContent } from "./AgentStatusView";

const COMPACT_OUTPUT = `⚠️  Alerts: 1 stuck (>15min), 1 failed, 1 needing approval

P0 🔄  T-1   implement-render       | generating |  3m ago | last: cat
P1 🔴  T-2   fix-tests              | STUCK 18m   | needs attention
P2 ❌  T-3   broken-card            | failed     | 2h ago
P1 ✅  T-4   done-card              | done       | 4m ago
P1 ⏸️  T-5   paused-card            | paused     | 10m ago
showing 5 of 5; no more pages
`;

function parsedReport(): AgentStatusReport {
  const report = parseAgentStatusOutput(COMPACT_OUTPUT);
  if (!report) throw new Error("expected report");
  return report;
}

describe("AgentStatusView parsing", () => {
  it("parses compact one-line agent rows", () => {
    const report = parsedReport();

    expect(report.alerts).toEqual({ stuck: 1, failed: 1, paused: 1 });
    expect(report.rows).toHaveLength(5);
    expect(report.rows[0]).toMatchObject({
      priority: "P0",
      cardId: "T-1",
      title: "implement-render",
      state: "running",
      age: "3m ago",
      ageMinutes: 3,
      lastTool: "cat",
    });
    expect(report.rows[1]).toMatchObject({
      priority: "P1",
      cardId: "T-2",
      state: "stuck",
      age: "18m",
      ageMinutes: 18,
    });
  });
});

describe("AgentStatusContent", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("filters rows by tab, priority, and age", () => {
    const report = parsedReport();

    expect(
      filterAgentStatusRows(report.rows, {
        tab: "stuck",
        priority: "P1",
        minAgeMinutes: 15,
      }).map((row) => row.cardId),
    ).toEqual(["T-2"]);

    expect(
      filterAgentStatusRows(report.rows, {
        tab: "running",
        priority: "P1",
        minAgeMinutes: null,
      }),
    ).toHaveLength(0);
  });

  it("renders sticky alerts when stuck or failed agents are present", () => {
    render(<AgentStatusContent report={parsedReport()} />);

    expect(
      screen.getByText("1 stuck, 1 failed, 1 needing approval"),
    ).toBeInTheDocument();
  });

  it("shows paused row under the Paused tab", async () => {
    const { user } = render(<AgentStatusContent report={parsedReport()} />);

    await user.click(screen.getByRole("tab", { name: /Paused 1/u }));

    await waitFor(() => {
      const table = screen.getByRole("table");
      expect(within(table).getByText("paused-card")).toBeInTheDocument();
      expect(
        within(table).queryByText("implement-render"),
      ).not.toBeInTheDocument();
    });
  });

  it("toggles agent row details with aria-expanded", async () => {
    const { user } = render(<AgentStatusContent report={parsedReport()} />);
    const toggle = screen.getByLabelText("Toggle details T-1");

    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByText("Last status update")).not.toBeInTheDocument();

    await user.click(toggle);

    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText("Last status update")).toBeInTheDocument();
  });

  it("dispatches action commands on button clicks", async () => {
    const onSubmitCommand = vi.fn((command: string): Promise<void> => {
      void command;
      return Promise.resolve();
    });
    render(
      <AgentStatusContent
        report={parsedReport()}
        onSubmitCommand={onSubmitCommand}
      />,
    );

    fireEvent.click(screen.getByLabelText("View pulse T-1"));
    await waitFor(() => {
      expect(onSubmitCommand).toHaveBeenCalledWith(
        formatAgentActionCommand("pulse", "T-1"),
      );
    });

    fireEvent.click(screen.getByText("Close"));
    fireEvent.click(screen.getByLabelText("View diff T-1"));
    await waitFor(() => {
      expect(onSubmitCommand).toHaveBeenCalledWith(
        formatAgentActionCommand("diff", "T-1"),
      );
    });

    fireEvent.click(screen.getByText("Close"));
    fireEvent.click(screen.getByLabelText("Steer T-1"));
    fireEvent.change(screen.getByLabelText("Steering message"), {
      target: { value: "Please check the failing test" },
    });
    fireEvent.click(screen.getByText("Send steer"));
    await waitFor(() => {
      expect(onSubmitCommand).toHaveBeenCalledWith(
        formatAgentActionCommand(
          "steer",
          "T-1",
          "Please check the failing test",
        ),
      );
    });

    fireEvent.click(screen.getByText("Close"));
    fireEvent.click(screen.getByLabelText("Cancel agent T-1"));
    fireEvent.click(screen.getByText("Confirm cancel"));
    await waitFor(() => {
      expect(onSubmitCommand).toHaveBeenCalledWith(
        formatAgentActionCommand(
          "cancel",
          "T-1",
          "Cancelled from agent status view.",
        ),
      );
    });
  });
});
