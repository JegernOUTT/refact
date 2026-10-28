import { describe, expect, test } from "vitest";
import { fireEvent, within } from "@testing-library/react";
import {
  createDefaultChatState,
  render,
  screen,
} from "../../../utils/test-utils";
import type {
  BackgroundAgentSummary,
  ChatMessages,
  ToolCall,
  ToolMessage,
} from "../../../services/refact/types";
import { ToolContent } from "../../ChatContent/ToolsContent";
import {
  AGENTIC_CARD_LABELS,
  AGENTIC_TOOL_NAMES,
  BOARD_RUNTIME_TOOL_NAMES,
  DEDICATED_TOOL_CARD_NAMES,
  agenticCardLabel,
  hasDedicatedToolCard,
  isAgenticToolName,
  lookupAgenticFamily,
} from "./agenticFamilies";
import { resumeReceipt } from "./resumeFixtures";

const CHAT_ID = "agentic-chat";
const CHILD_ID = "bgagent-0f673107-f6a5-45b0-a41f-1cbe15e76a2f";

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
  extra?: Record<string, unknown>,
): ToolMessage {
  return {
    role: "tool",
    tool_call_id: `call-${name}`,
    content,
    tool_failed: failed,
    extra,
  };
}

function childAgent(): BackgroundAgentSummary {
  return {
    agent_id: CHILD_ID,
    parent_chat_id: CHAT_ID,
    child_chat_id: "child-chat",
    kind: "subagent",
    status: "running",
    title: "Subagent: Migrate the exec family",
    progress: null,
    step_count: 3,
    last_activity: null,
    target_files: [],
    edited_files: [],
    diff_summary: null,
    conflict_summary: null,
    result_summary: null,
    error: null,
    started_at: null,
    finished_at: null,
    change_seq: 1,
  };
}

function renderTools(messages: ChatMessages, calls: ToolCall[]) {
  const chat = createDefaultChatState();
  const runtime = chat.threads[chat.current_thread_id];
  runtime.thread.id = CHAT_ID;
  runtime.thread.messages = messages;
  runtime.streaming = false;
  runtime.waiting_for_response = false;
  runtime.background_agents = { [CHILD_ID]: childAgent() };
  chat.current_thread_id = CHAT_ID;
  chat.open_thread_ids = [CHAT_ID];
  chat.threads = { [CHAT_ID]: runtime };
  return render(<ToolContent toolCalls={calls} threadId={CHAT_ID} />, {
    preloadedState: { chat },
  });
}

function open() {
  fireEvent.click(screen.getByTestId("transcript-toggle"));
}

const INVENTORY = Array.from(
  new Set([...AGENTIC_TOOL_NAMES, ...BOARD_RUNTIME_TOOL_NAMES]),
).filter((name) => isAgenticToolName(name));

describe("AgenticTool inventory", () => {
  test("every family and tool name owns a label", () => {
    const families = [...AGENTIC_TOOL_NAMES, "get_plan"].map(
      (name) => lookupAgenticFamily(name)?.family,
    );
    for (const family of Object.keys(AGENTIC_CARD_LABELS)) {
      expect(families, family).toContain(family);
    }
    for (const name of AGENTIC_TOOL_NAMES) {
      expect(agenticCardLabel(name), name).not.toBeNull();
    }
    for (const name of BOARD_RUNTIME_TOOL_NAMES) {
      expect(hasDedicatedToolCard(name), name).toBe(true);
    }
  });

  test.each(
    INVENTORY.filter((name) => !DEDICATED_TOOL_CARD_NAMES.includes(name)),
  )("%s renders exactly one transcript item with raw once", (name) => {
    const { unmount } = renderTools(
      [toolResult(name, "# Outcome\n\n- Status: running\n\nrecorded")],
      [toolCall(name, { agent_id: "abc", card_id: "T-1", text: "hello" })],
    );
    expect(screen.getAllByTestId("agentic-tool")).toHaveLength(1);
    expect(screen.getAllByTestId("transcript-row")).toHaveLength(1);
    expect(screen.queryByTestId("generic-tool")).toBeNull();
    expect(screen.queryByText("More details")).toBeNull();
    expect(screen.getByTestId("transcript-toggle")).toHaveAttribute(
      "aria-expanded",
      "false",
    );
    open();
    expect(screen.getAllByTestId("transcript-raw")).toHaveLength(1);
    expect(document.querySelector("details")).toBeNull();
    unmount();
  });
});

describe("AgenticTool agent_message", () => {
  test("names the recipient, stays quiet and shows Sent", () => {
    renderTools(
      [toolResult("agent_message", `Message queued for ${CHILD_ID}.`)],
      [toolCall("agent_message", { to: CHILD_ID, text: "Rebase **please**" })],
    );
    expect(screen.getByTestId("transcript-verb")).toHaveTextContent("Message");
    expect(screen.getByTestId("transcript-target")).toHaveTextContent(
      "Migrate the exec family",
    );
    expect(screen.getByTestId("transcript-meta")).toHaveTextContent(
      "Rebase **please**",
    );
    expect(screen.queryByTestId("transcript-status-pill")).toBeNull();
    expect(screen.queryByText("Queued")).toBeNull();
    open();
    const facts = screen.getByTestId("transcript-facts");
    expect(within(facts).getByText("Sent")).toBeInTheDocument();
    const body = screen.getByTestId("agentic-tool-message");
    expect(body.querySelector("strong")).toHaveTextContent("please");
    expect(
      screen.getByRole("button", { name: "Open child trajectory" }),
    ).toBeInTheDocument();
  });

  test("a failed send shows the Failed pill and the reason", () => {
    renderTools(
      [toolResult("agent_message", "recipient not found", true)],
      [toolCall("agent_message", { to: "ghost", text: "hi" })],
    );
    expect(screen.getByTestId("transcript-status-pill")).toHaveTextContent(
      "Failed",
    );
    open();
    expect(screen.getByRole("alert")).toHaveTextContent("recipient not found");
  });
});

describe("AgenticTool other agentic rows", () => {
  test("agents_overview lists agents with counts in the target", () => {
    renderTools(
      [
        toolResult(
          "agents_overview",
          `# Agents Overview\n\n- ${CHILD_ID.slice(
            0,
            16,
          )} — Subagent: Migrate the exec family — 🟢 running — step 3 — tool: cat`,
        ),
      ],
      [toolCall("agents_overview")],
    );
    expect(screen.getByTestId("transcript-target")).toHaveTextContent(
      "1 running",
    );
    open();
    const list = screen.getByTestId("agentic-tool-agents");
    expect(list).toHaveTextContent("Migrate the exec family");
    expect(list).toHaveTextContent("step 3");
    expect(screen.queryByText(/Historical snapshot/)).toBeNull();
    expect(screen.queryByText(/Roster snapshot/)).toBeNull();
  });

  test("get_goal renders the goal markdown and criteria", () => {
    renderTools(
      [
        toolResult(
          "get_goal",
          JSON.stringify({
            goal: {
              content: "Ship the pond",
              status: "paused",
              version: 1,
              delta_count: 0,
              turns_used: 1,
              tokens_used: 3210,
              gaps: [],
              criteria: [{ id: "C1", text: "Tests pass" }],
            },
          }),
        ),
      ],
      [toolCall("get_goal")],
    );
    expect(screen.getByTestId("transcript-verb")).toHaveTextContent("Goal");
    expect(screen.getByTestId("transcript-target")).toHaveTextContent(
      "Ship the pond",
    );
    open();
    expect(screen.getByLabelText("Criteria")).toHaveTextContent("Tests pass");
    expect(screen.getByTestId("transcript-facts")).toHaveTextContent("3210");
  });

  test("agent_resume keeps the typed receipt and the open-child action", () => {
    renderTools(
      [toolResult("agent_resume", JSON.stringify(resumeReceipt()))],
      [toolCall("agent_resume", { action: "inspect", agent_id: "x" })],
    );
    expect(screen.getByTestId("transcript-status-pill")).toHaveTextContent(
      "Cap reached",
    );
    open();
    expect(screen.getByLabelText("Execution cap")).toHaveTextContent("20 / 20");
    expect(
      screen.getByRole("button", { name: "Open child trajectory" }),
    ).toBeInTheDocument();
  });

  test("unrecorded calls show the shared No result pill without a panel claim", () => {
    renderTools([], [toolCall("agent_status", { agent_id: CHILD_ID })]);
    expect(screen.getByTestId("transcript-status-pill")).toHaveTextContent(
      "No result",
    );
    expect(screen.getByTestId("transcript-target")).toHaveTextContent(
      "Migrate the exec family",
    );
  });
});
