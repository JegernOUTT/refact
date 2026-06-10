import { describe, expect, it } from "vitest";
import {
  createDefaultChatState,
  fireEvent,
  render,
  screen,
} from "../../utils/test-utils";
import { FinalReportView } from "./FinalReportView";
import { ToolContent } from "./ToolsContent";
import type {
  ChatMessages,
  ToolCall,
  ToolMessage,
} from "../../services/refact/types";

const structuredPayload = JSON.stringify({
  summary: "Implemented **structured** final report rendering.",
  success: true,
  files_changed: ["src/components/ChatContent/FinalReportView.tsx"],
  tests_added_or_updated: ["FinalReportView.test.tsx"],
  verification: [
    {
      command: "npm run test -- FinalReportView --run",
      exit_code: 0,
      passed: true,
      output_tail: "test passed",
    },
    {
      command: "npm run lint",
      exit_code: 1,
      passed: false,
      output_tail: "lint failed",
    },
  ],
  followup_cards: [
    {
      title: "Add create-card actions",
      priority: "P2",
      instructions: "Add optional follow-up card creation controls later.",
    },
  ],
  risks: ["Renderer only handles the current structured schema."],
  assumptions: ["Legacy markdown remains available."],
});

const unsafeStructuredPayload = JSON.stringify({
  summary: [
    "Rendered untrusted markdown.",
    "<script>window.x=1</script>",
    '<img onerror="alert(1)" src=x>',
  ].join("\n\n"),
  success: true,
  files_changed: [],
  tests_added_or_updated: [],
  verification: [],
  followup_cards: [],
  risks: [],
  assumptions: [],
});

function makeToolCall(): ToolCall {
  return {
    id: "call-task-agent-finish",
    index: 0,
    type: "function",
    function: {
      name: "agent_finish",
      arguments: "{}",
    },
  };
}

function makeToolMessage(content: string): ToolMessage {
  return {
    role: "tool",
    tool_call_id: "call-task-agent-finish",
    content,
    tool_failed: false,
  };
}

function renderFinalReportTool(content: string) {
  const chat = createDefaultChatState();
  const runtime = chat.threads[chat.current_thread_id];
  // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition
  if (!runtime) throw new Error("missing test thread");
  runtime.thread.messages = [makeToolMessage(content)] as ChatMessages;
  return render(<ToolContent toolCalls={[makeToolCall()]} />, {
    preloadedState: { chat },
  });
}

describe("FinalReportView", () => {
  it("renders structured payload sections", () => {
    render(<FinalReportView content={structuredPayload} title="Card T-1" />);
    expect(screen.getByText("Card T-1")).toBeInTheDocument();
    expect(screen.getByText(/Success/)).toBeInTheDocument();
    expect(screen.getByText("structured")).toBeInTheDocument();
    expect(
      screen.getByText("src/components/ChatContent/FinalReportView.tsx"),
    ).toBeInTheDocument();
    expect(screen.getByText("FinalReportView.test.tsx")).toBeInTheDocument();
    expect(screen.getByText("Followup cards")).toBeInTheDocument();
    expect(screen.getByText("Add create-card actions")).toBeInTheDocument();
  });

  it("does not render raw scripts or inline image handlers from summary markdown", () => {
    const { container } = render(
      <FinalReportView content={unsafeStructuredPayload} />,
    );

    expect(container.querySelector("script")).not.toBeInTheDocument();
    expect(container.querySelector("img[onerror]")).not.toBeInTheDocument();
  });

  it("shows verification pass and fail states", () => {
    render(<FinalReportView content={structuredPayload} />);
    expect(screen.getByText(/Success/)).toBeInTheDocument();
    expect(
      screen.getByText("npm run test -- FinalReportView --run"),
    ).toBeInTheDocument();
    expect(screen.getByText("npm run lint")).toBeInTheDocument();
  });

  it("renders legacy string payload without error", () => {
    render(<FinalReportView content="Legacy report\n\nStill readable." />);
    expect(screen.getByText(/Legacy report/)).toBeInTheDocument();
    expect(screen.getByText(/Still readable/)).toBeInTheDocument();
  });

  it("renders minimal structured payload", () => {
    render(
      <FinalReportView
        content={JSON.stringify({ summary: "Minimal report", success: true })}
      />,
    );

    expect(screen.getByText("Minimal report")).toBeInTheDocument();
    expect(screen.getByText(/Success/)).toBeInTheDocument();
    expect(screen.getByText("Files changed")).toBeInTheDocument();
    expect(screen.getByText("Tests added or updated")).toBeInTheDocument();
    expect(screen.getByText("Verification")).toBeInTheDocument();
    expect(screen.getByText("Followup cards")).toBeInTheDocument();
    expect(screen.getByText("Risks")).toBeInTheDocument();
    expect(screen.getByText("Assumptions")).toBeInTheDocument();
    expect(screen.getAllByText("None")).toHaveLength(6);
  });

  it("renders null optional fields as empty sections", () => {
    render(
      <FinalReportView
        content={JSON.stringify({
          summary: "Null optional fields report",
          success: false,
          files_changed: null,
          tests_added_or_updated: null,
          verification: null,
          followup_cards: null,
          risks: null,
          assumptions: null,
        })}
      />,
    );

    expect(screen.getByText("Null optional fields report")).toBeInTheDocument();
    expect(screen.getByText(/Failed/)).toBeInTheDocument();
    expect(screen.getByText("Files changed")).toBeInTheDocument();
    expect(screen.getByText("Tests added or updated")).toBeInTheDocument();
    expect(screen.getByText("Verification")).toBeInTheDocument();
    expect(screen.getByText("Followup cards")).toBeInTheDocument();
    expect(screen.getByText("Risks")).toBeInTheDocument();
    expect(screen.getByText("Assumptions")).toBeInTheDocument();
    expect(screen.getAllByText("None")).toHaveLength(6);
  });

  it("renders followup cards read-only", () => {
    render(<FinalReportView content={structuredPayload} />);
    expect(screen.getByText("Add create-card actions")).toBeInTheDocument();
    expect(screen.getByText("P2")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /create/i }),
    ).not.toBeInTheDocument();
    expect(screen.queryByText("+")).not.toBeInTheDocument();
  });

  it("keeps verification output tails expandable", () => {
    const { container } = render(
      <FinalReportView content={structuredPayload} />,
    );
    const toggle = screen.getByRole("button", {
      name: /npm run test -- FinalReportView --run/u,
    });

    expect(container.querySelector("details")).not.toBeInTheDocument();
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByText("test passed")).not.toBeInTheDocument();

    fireEvent.click(toggle);
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText("test passed")).toBeInTheDocument();

    fireEvent.click(toggle);
    expect(toggle).toHaveAttribute("aria-expanded", "false");
  });

  it("renders risks and assumptions as animated disclosure buttons", () => {
    render(<FinalReportView content={structuredPayload} />);

    const risks = screen.getByRole("button", { name: /Risks \(1\)/u });
    const assumptions = screen.getByRole("button", {
      name: /Assumptions \(1\)/u,
    });

    expect(risks).toHaveAttribute("aria-expanded", "false");
    expect(assumptions).toHaveAttribute("aria-expanded", "false");

    fireEvent.click(risks);
    expect(risks).toHaveAttribute("aria-expanded", "true");
    expect(
      screen.getByText("Renderer only handles the current structured schema."),
    ).toBeInTheDocument();
  });

  it("renders agent_finish reports inside a ToolCard wrapper", () => {
    renderFinalReportTool(structuredPayload);

    expect(screen.getByTestId("final-report-tool")).toBeInTheDocument();
    expect(screen.getByText("Task agent final report")).toBeInTheDocument();
    expect(screen.getByTestId("final-report-view")).toBeInTheDocument();
    expect(screen.getByText("Final Report")).toBeInTheDocument();
  });
});
