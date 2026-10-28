import { describe, expect, it } from "vitest";
import {
  createDefaultChatState,
  fireEvent,
  render,
  screen,
} from "../../utils/test-utils";
import { FinalReportTool, FinalReportView } from "./FinalReportView";
import { finalReportPresentation } from "./finalReportJson";
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
  return render(
    <FinalReportTool
      content={content}
      hasResult
      rawContent={content}
      toolCall={makeToolCall()}
    />,
    { preloadedState: { chat } },
  );
}

describe("FinalReportView", () => {
  it("renders the structured payload as sections", () => {
    render(<FinalReportView content={structuredPayload} />);
    expect(screen.getByTestId("final-report-view")).toBeInTheDocument();
    expect(screen.getByText("structured").tagName).toBe("STRONG");
    expect(screen.getByText("FinalReportView.test.tsx")).toBeInTheDocument();
    expect(screen.getByText("Followup cards")).toBeInTheDocument();
    expect(
      screen.getByText("Add create-card actions", { exact: false }),
    ).toBeInTheDocument();
    expect(
      screen.getByText("Renderer only handles the current structured schema."),
    ).toBeInTheDocument();
    expect(
      screen.getByText("Legacy markdown remains available."),
    ).toBeInTheDocument();
  });

  it("does not render raw scripts or inline image handlers from summary markdown", () => {
    const { container } = render(
      <FinalReportView content={unsafeStructuredPayload} />,
    );
    expect(container.querySelector("script")).not.toBeInTheDocument();
    expect(container.querySelector("img[onerror]")).not.toBeInTheDocument();
  });

  it("shows verification commands with their output tails", () => {
    render(<FinalReportView content={structuredPayload} />);
    expect(
      screen.getByText("npm run test -- FinalReportView --run"),
    ).toBeInTheDocument();
    expect(screen.getByText("npm run lint")).toBeInTheDocument();
    expect(screen.getByText("lint failed")).toBeInTheDocument();
  });

  it("renders a legacy string payload as markdown", () => {
    render(<FinalReportView content={"# Legacy report\n\nStill readable."} />);
    expect(
      screen.getByRole("heading", { name: "Legacy report" }),
    ).toBeInTheDocument();
    expect(screen.getByText(/Still readable/)).toBeInTheDocument();
  });

  it("omits empty optional sections", () => {
    render(
      <FinalReportView
        content={JSON.stringify({
          summary: "Null optional fields report",
          success: false,
          files_changed: null,
          verification: null,
          risks: null,
        })}
      />,
    );
    expect(screen.getByText("Null optional fields report")).toBeInTheDocument();
    expect(screen.queryByText("Verification")).toBeNull();
    expect(screen.queryByText("Risks")).toBeNull();
  });

  it("renders full followup instructions inside the clamp", () => {
    const instructions = `Keep every instruction ${"x".repeat(
      9000,
    )}FOLLOWUP_TAIL`;
    const { container } = render(
      <FinalReportView
        content={JSON.stringify({
          summary: "Long followup report",
          success: true,
          followup_cards: [
            { title: "Add create-card actions", priority: "P2", instructions },
          ],
        })}
      />,
    );
    expect(screen.getByTestId("transcript-clamp")).toBeInTheDocument();
    expect(container.textContent).toContain("FOLLOWUP_TAIL");
    expect(screen.queryByRole("button", { name: /create/i })).toBeNull();
  });
});

describe("FinalReportTool", () => {
  it("renders agent_finish as one Report item with summary target and verification meta", () => {
    const { container } = renderFinalReportTool(structuredPayload);
    expect(screen.getByTestId("final-report-tool")).toHaveAttribute(
      "data-transcript-item",
      "true",
    );
    expect(screen.getByTestId("transcript-verb")).toHaveTextContent("Report");
    expect(screen.getByTestId("transcript-target")).toHaveTextContent(
      "Implemented **structured** final report rendering.",
    );
    expect(screen.getByTestId("transcript-meta")).toHaveTextContent(
      "1/2 checks reported passed",
    );
    expect(
      container.querySelectorAll('[data-transcript-item="true"]'),
    ).toHaveLength(1);
    expect(container.textContent).not.toContain("Technical details");
  });

  it("starts collapsed unless it is the latest report", async () => {
    const { user, unmount } = renderFinalReportTool(structuredPayload);
    const toggle = screen.getByTestId("transcript-toggle");
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    await user.click(toggle);
    expect(screen.getByTestId("final-report-view")).toBeInTheDocument();
    expect(screen.getByText("Success (self-reported)")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Copy report" }),
    ).toBeInTheDocument();
    unmount();
    render(
      <FinalReportTool
        content={structuredPayload}
        hasResult
        latest
        toolCall={makeToolCall()}
      />,
    );
    expect(screen.getByTestId("transcript-toggle")).toHaveAttribute(
      "aria-expanded",
      "true",
    );
  });

  it("opens the most recent final report in the thread by default", () => {
    const chat = createDefaultChatState();
    const runtime = chat.threads[chat.current_thread_id];
    runtime.thread.messages = [
      {
        role: "assistant",
        content: "",
        tool_calls: [makeToolCall()],
      },
      makeToolMessage(structuredPayload),
    ] as ChatMessages;
    render(
      <FinalReportTool
        content={structuredPayload}
        hasResult
        toolCall={makeToolCall()}
      />,
      { preloadedState: { chat } },
    );
    expect(screen.getByTestId("transcript-toggle")).toHaveAttribute(
      "aria-expanded",
      "true",
    );
  });

  it("legacy markdown stays recoverable without outcome inference", () => {
    renderFinalReportTool("PASS! verified by prose only");
    expect(screen.getByTestId("transcript-status-pill")).toHaveTextContent(
      "Outcome unconfirmed",
    );
    fireEvent.click(screen.getByTestId("transcript-toggle"));
    expect(
      screen.getByText("No structured outcome or verification evidence"),
    ).toBeInTheDocument();
    expect(
      screen.getByText("PASS! verified by prose only", { selector: "p" }),
    ).toBeInTheDocument();
  });

  it("keeps raw arguments and response", () => {
    renderFinalReportTool(structuredPayload);
    fireEvent.click(screen.getByTestId("transcript-toggle"));
    fireEvent.click(screen.getByRole("button", { name: /^Raw/ }));
    expect(screen.getByTestId("transcript-raw")).toHaveTextContent(
      '"output_tail": "lint failed"',
    );
  });

  it.each([
    [null, false, "Awaiting report"],
    ["PASS!", false, "Outcome unconfirmed"],
    ['{"summary":"failed","success":false}', false, "Reported failure"],
    ['{"summary":"ok","success":true}', false, "Reported success"],
    ['{"summary":"ok","success":true}', true, "Request failed"],
    ['{"summary":"bad schema","success":"true"}', false, "Outcome unconfirmed"],
  ] as const)(
    "presentation separates outcome and transport for %s",
    (content, failed, stateLabel) => {
      const presentation = finalReportPresentation(content, failed);
      expect(presentation.stateLabel).toBe(stateLabel);
      expect(presentation.stateLabel).not.toMatch(/completed|verified/i);
      expect(presentation.tone).not.toBe("positive");
    },
  );

  it("contradictory or unrun checks are not counted as passed", () => {
    const payload = JSON.stringify({
      summary: "Attempted checks",
      success: true,
      verification: [
        {
          command: "npm test",
          passed: true,
          exit_code: 1,
          output_tail: "failed",
        },
        {
          command: "npm run lint",
          passed: true,
          exit_code: null,
          output_tail: "not run",
        },
        { command: "", passed: true, exit_code: 0, output_tail: "" },
      ],
    });
    expect(finalReportPresentation(payload).facts).toContainEqual({
      label: "Verification",
      value: "0/3 checks reported passed",
    });
  });
});
