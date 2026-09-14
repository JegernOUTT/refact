import { describe, expect, test } from "vitest";
import { fireEvent } from "@testing-library/react";
import {
  createDefaultChatState,
  render,
  screen,
} from "../../../utils/test-utils";
import type {
  ChatMessages,
  ToolCall,
  ToolMessage,
} from "../../../services/refact/types";
import { ToolContent } from "../../ChatContent/ToolsContent";
import {
  AGENTIC_TOOL_NAMES,
  BOARD_RUNTIME_TOOL_NAMES,
  DEDICATED_TOOL_CARD_NAMES,
} from "./agenticFamilies";
import { PAGE_CHARS } from "./PagedText";
import { resumeReceipt } from "./resumeFixtures";
import { launchPreviewCall, launchPreviewReceipt } from "./launchFixtures";

const CHAT_ID = "agentic-chat";

function toolCall(name: string, args: Record<string, unknown> = {}): ToolCall {
  return {
    id: `call-${name}`,
    index: 0,
    type: "function",
    function: { name, arguments: JSON.stringify(args) },
  };
}

function toolResult(
  name: string,
  content: string,
  failed = false,
): ToolMessage {
  return {
    role: "tool",
    tool_call_id: `call-${name}`,
    content,
    tool_failed: failed,
  };
}

function renderTools(messages: ChatMessages, calls: ToolCall[]) {
  const chat = createDefaultChatState();
  const runtime = chat.threads[chat.current_thread_id];
  runtime.thread.id = CHAT_ID;
  runtime.thread.messages = messages;
  runtime.streaming = false;
  runtime.waiting_for_response = false;
  chat.current_thread_id = CHAT_ID;
  chat.open_thread_ids = [CHAT_ID];
  chat.threads = { [CHAT_ID]: runtime };

  return render(<ToolContent toolCalls={calls} threadId={CHAT_ID} />, {
    preloadedState: { chat },
  });
}

describe("AgenticTool registry wiring", () => {
  test.each([
    "agent_message",
    "agent_peer_grant",
    "agent_steer",
    "task_broadcast",
    "progress_report",
  ])("routes %s to a dedicated interaction card", (name) => {
    renderTools(
      [toolResult(name, "Legacy response")],
      [
        toolCall(name, {
          text: "hello",
          message: "hello",
          status_line: "hello",
        }),
      ],
    );
    expect(
      screen.getByLabelText(
        name === "progress_report"
          ? "Progress receipt"
          : "Agent message receipt",
      ),
    ).toBeInTheDocument();
    expect(screen.queryByTestId("agentic-tool-body")).toBeNull();
    expect(screen.queryByTestId("generic-tool")).toBeNull();
  });

  test.each([
    ["set_goal", '{"version":1,"supersedes":null}'],
    ["update_goal", '{"seq":2,"truncated":false}'],
    ["get_goal", '{"goal":null}'],
    ["validate_goal", "No active goal to validate."],
    [
      "pause_goal",
      "Goal pursuit paused. The user can resume it with goal_control(resume).",
    ],
    [
      "snooze_goal",
      "Goal pursuit snoozed for 4 minutes; it resumes automatically afterwards or on the next user message.",
    ],
  ])("routes %s to its dedicated goal receipt", (name, content) => {
    renderTools([toolResult(name, content)], [toolCall(name)]);
    expect(screen.getByLabelText("Goal receipt")).toBeInTheDocument();
    expect(screen.queryByTestId("agentic-tool-body")).toBeNull();
    expect(screen.queryByTestId("generic-tool")).toBeNull();
  });

  test.each(AGENTIC_TOOL_NAMES.map((name) => [name] as const))(
    "%s reaches the agentic tool card",
    (name) => {
      const call = toolCall(name, { agent_id: "abc", text: "hello" });
      const { unmount } = renderTools(
        [toolResult(name, "- Status: running")],
        [call],
      );
      expect(screen.getByTestId("agentic-tool")).toBeInTheDocument();
      expect(screen.queryByTestId("generic-tool")).toBeNull();
      unmount();
    },
  );

  test.each(
    BOARD_RUNTIME_TOOL_NAMES.filter(
      (name) => !DEDICATED_TOOL_CARD_NAMES.includes(name),
    ).map((name) => [name] as const),
  )("board tool %s renders a family card, never raw output", (name) => {
    const { unmount } = renderTools(
      [toolResult(name, "# Outcome\n\nrecorded")],
      [toolCall(name, { task_id: "task-1", card_id: "T-1" })],
    );
    expect(screen.getByTestId("agentic-tool")).toBeInTheDocument();
    expect(screen.queryByTestId("generic-tool")).toBeNull();
    unmount();
  });

  test("long generic results are paged instead of rendered whole", () => {
    const body = "x".repeat(PAGE_CHARS * 2 + 10);
    renderTools(
      [toolResult("task_verify_card", body)],
      [toolCall("task_verify_card", { card_id: "T-1" })],
    );
    const shown = screen.getByTestId("agentic-tool-body");
    expect(shown.textContent?.length).toBe(PAGE_CHARS);
    expect(screen.getByRole("status")).toHaveTextContent("Section 1 of 3");
    const next = screen.getByRole("button", { name: "Next section" });
    fireEvent.click(next);
    expect(screen.getByRole("status")).toHaveTextContent("Section 2 of 3");
    fireEvent.click(next);
    expect(screen.getByTestId("agentic-tool-body").textContent?.length).toBe(
      10,
    );
    expect(next).toBeDisabled();
  });

  test("renders sender to recipient body for agent_message", () => {
    renderTools(
      [toolResult("agent_message", "Message queued for peer-1.")],
      [toolCall("agent_message", { to: "peer-1", text: "Rebase please" })],
    );

    expect(screen.getByTestId("agentic-tool-message")).toBeInTheDocument();
    expect(screen.getByText("Rebase please")).toBeInTheDocument();
    expect(screen.getByText("peer-1")).toBeInTheDocument();
  });

  test("renders agent rows for agents_overview", () => {
    const output = [
      "# Agents Overview",
      "",
      "- abc12345 — Fix the pond — running — step 3",
    ].join("\n");

    renderTools(
      [toolResult("agents_overview", output)],
      [toolCall("agents_overview")],
    );

    expect(screen.getByTestId("agentic-tool-agents")).toBeInTheDocument();
    expect(screen.getByText("Fix the pond")).toBeInTheDocument();
  });

  test("shows pending state when no result arrived", () => {
    renderTools([], [toolCall("agent_wait", { agent_id: "abc" })]);

    expect(screen.getByTestId("agentic-tool-pending")).toBeInTheDocument();
  });

  test("routes approval receipts to the dedicated decision card", () => {
    renderTools(
      [
        toolResult(
          "agent_approval",
          JSON.stringify({
            request_id: "request",
            decision: "approved",
            scope: "once",
          }),
        ),
      ],
      [
        toolCall("agent_approval", {
          action: "approve",
          request_id: "request",
        }),
      ],
    );
    expect(screen.getByLabelText("Child command approval")).toHaveTextContent(
      "Single execution only",
    );
    expect(screen.queryByTestId("agentic-tool-body")).not.toBeInTheDocument();
  });

  test("routes resume receipts to the typed cap and continuation card", () => {
    renderTools(
      [toolResult("agent_resume", JSON.stringify(resumeReceipt()))],
      [
        toolCall("agent_resume", {
          action: "inspect",
          agent_id: "stopped-child",
        }),
      ],
    );
    expect(screen.getByLabelText("Child resume")).toHaveTextContent(
      "Execution cap reached",
    );
    expect(screen.queryByTestId("agentic-tool-body")).toBeNull();
  });

  test("shows error block for failed calls", () => {
    renderTools(
      [toolResult("agent_result", "Background agent not found.", true)],
      [toolCall("agent_result", { agent_id: "abc" })],
    );

    expect(screen.getByTestId("agentic-tool-error")).toBeInTheDocument();
  });

  test("routes subagent previews through the dedicated context card", () => {
    renderTools([launchPreviewReceipt()], [launchPreviewCall]);
    expect(screen.getByLabelText("Child launch preview")).toHaveTextContent(
      "Preview only",
    );
    expect(screen.queryByTestId("generic-tool")).toBeNull();
  });

  test("keeps subagent on the background agent card", () => {
    renderTools(
      [toolResult("subagent", "Started background subagent.")],
      [toolCall("subagent", { task: "Do work", expected_result: "done" })],
    );

    expect(screen.queryByTestId("agentic-tool")).toBeNull();
  });

  test("renders raw arguments only inside a collapsed disclosure", () => {
    renderTools(
      [toolResult("agent_cancel", "✓ Cancel requested.")],
      [toolCall("agent_cancel", { agent_id: "abc", reason: "stale" })],
    );

    const raw = screen.getByTestId("agentic-tool-raw-args");
    expect(raw.closest("details")?.open).toBe(false);
  });

  test.each([
    ["set_plan", { version: 1, supersedes: null }],
    ["update_plan", { seq: 2, truncated: false }],
    [
      "get_plan",
      {
        plan: {
          content: "Accepted intent",
          version: 1,
          delta_count: 2,
          mode: "agent",
        },
      },
    ],
  ])("routes %s to the dedicated plan card", (name, payload) => {
    renderTools([toolResult(name, JSON.stringify(payload))], [toolCall(name)]);
    expect(screen.getByLabelText("Plan receipt")).toBeInTheDocument();
    expect(screen.queryByTestId("agentic-tool-body")).toBeNull();
    expect(screen.queryByTestId("generic-tool")).toBeNull();
  });
});
