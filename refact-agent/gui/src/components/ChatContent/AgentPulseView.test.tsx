import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "../../utils/test-utils";
import {
  DEFAULT_CANCEL_REASON,
  DEFAULT_PAUSE_REASON,
  formatAgentActionCommand,
} from "./AgentStatusModel";
import {
  type AgentPulseReport,
  parseAgentPulseOutput,
} from "./AgentPulseModel";
import { AgentPulseContent } from "./AgentPulseView";

const PULSE_OUTPUT =
  `# Agent Pulse: T-29

**Card:** check-agents redesign
**State:** 🔄 generating response
**Last activity:** 3m ago
**Tokens used:** ~38k / 200k
**Currently editing:** src/tools/tool_task_check_agents.rs

## Last assistant message
> Adding sticky alerts logic...

## Last tool call
` +
  '`patch(path="src/tools/tool_task_check_agents.rs")`' +
  `
`;

function parsedReport(): AgentPulseReport {
  const report = parseAgentPulseOutput(PULSE_OUTPUT);
  if (!report) throw new Error("expected pulse report");
  return report;
}

describe("AgentPulseView parsing", () => {
  it("parses agent pulse markdown", () => {
    expect(parsedReport()).toMatchObject({
      cardId: "T-29",
      cardTitle: "check-agents redesign",
      state: "🔄 generating response",
      stateKind: "running",
      lastActivity: "3m ago",
      tokens: "~38k / 200k",
      currentlyEditing: "src/tools/tool_task_check_agents.rs",
      lastAssistantMessage: "Adding sticky alerts logic...",
      lastToolCall: 'patch(path="src/tools/tool_task_check_agents.rs")',
    });
  });
});

describe("AgentPulseContent", () => {
  it("renders pulse output", () => {
    render(<AgentPulseContent report={parsedReport()} />);

    expect(screen.getByText("Pulse: T-29")).toBeInTheDocument();
    expect(screen.getByText("~38k / 200k")).toBeInTheDocument();
    expect(
      screen.getByText("src/tools/tool_task_check_agents.rs"),
    ).toBeInTheDocument();
    expect(
      screen.getByText("Adding sticky alerts logic..."),
    ).toBeInTheDocument();
    expect(
      screen.getByText('patch(path="src/tools/tool_task_check_agents.rs")'),
    ).toBeInTheDocument();
  });

  it("dispatches action commands", async () => {
    const onSubmitCommand = vi.fn((command: string): Promise<void> => {
      void command;
      return Promise.resolve();
    });
    render(
      <AgentPulseContent
        report={parsedReport()}
        onSubmitCommand={onSubmitCommand}
      />,
    );

    fireEvent.click(screen.getByText("Pause"));
    await waitFor(() => {
      expect(onSubmitCommand).toHaveBeenCalledWith(
        formatAgentActionCommand("pause", "T-29", DEFAULT_PAUSE_REASON),
      );
    });

    fireEvent.click(screen.getByText("Close"));
    fireEvent.click(screen.getByText("Diff"));
    await waitFor(() => {
      expect(onSubmitCommand).toHaveBeenCalledWith(
        formatAgentActionCommand("diff", "T-29"),
      );
    });

    fireEvent.click(screen.getByText("Close"));
    fireEvent.click(screen.getByText("Steer"));
    fireEvent.change(screen.getByLabelText("Steering message"), {
      target: { value: "Please add the renderer tests" },
    });
    fireEvent.click(screen.getByText("Send steer"));
    await waitFor(() => {
      expect(onSubmitCommand).toHaveBeenCalledWith(
        formatAgentActionCommand(
          "steer",
          "T-29",
          "Please add the renderer tests",
        ),
      );
    });

    fireEvent.click(screen.getByText("Close"));
    fireEvent.click(screen.getByText("Cancel"));
    fireEvent.click(screen.getByText("Confirm cancel"));
    await waitFor(() => {
      expect(onSubmitCommand).toHaveBeenCalledWith(
        formatAgentActionCommand("cancel", "T-29", DEFAULT_CANCEL_REASON),
      );
    });
  });
});
