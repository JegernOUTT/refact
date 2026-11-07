import { afterEach, describe, expect, it, vi } from "vitest";
import { act as rtlAct } from "@testing-library/react";
import {
  createDefaultChatState,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "../../utils/test-utils";
import * as chatCommands from "../../services/refact/chatCommands";
import type { ToolCall, ToolMessage } from "../../services/refact/types";
import { applyChatEvent } from "../../features/Chat/Thread";
import { CollapsibleStoreProvider } from "./useStoredOpen";
import {
  DEFAULT_CANCEL_REASON,
  DEFAULT_PAUSE_REASON,
  formatAgentActionCommand,
} from "./AgentStatusModel";
import {
  type AgentPulseReport,
  parseAgentPulseOutput,
} from "./AgentPulseModel";
import { AgentPulseContent, AgentPulseView } from "./AgentPulseView";

const act = rtlAct as unknown as (callback: () => void) => void;

const TOOL_CALL: ToolCall = {
  id: "pulse-call",
  index: 0,
  function: { name: "agent_pulse", arguments: '{"card_id":"T-29"}' },
};

function pulseMessage(content: string, tool_failed = false): ToolMessage {
  return {
    role: "tool",
    message_id: "pulse-result",
    tool_call_id: "pulse-call",
    content,
    tool_failed,
  };
}

function renderPulse(
  content: string | null,
  toolFailed = false,
  waiting = false,
) {
  const chat = createDefaultChatState();
  const threadId = chat.current_thread_id;
  chat.threads[threadId].thread.messages =
    content === null
      ? [{ role: "assistant", content: "", tool_calls: [TOOL_CALL] }]
      : [pulseMessage(content, toolFailed)];
  chat.threads[threadId].waiting_for_response = waiting;
  return {
    ...render(<AgentPulseView toolCall={TOOL_CALL} />, {
      preloadedState: { chat },
    }),
    threadId,
  };
}

afterEach(() => {
  vi.restoreAllMocks();
});

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
  it("dispatches action commands", async () => {
    const onSubmitCommand = vi
      .spyOn(chatCommands, "sendChatCommand")
      .mockResolvedValue(undefined);
    renderPulse(PULSE_OUTPUT);
    const outer = screen.getByTestId("transcript-toggle");
    fireEvent.click(outer);
    expect(onSubmitCommand).not.toHaveBeenCalled();

    fireEvent.click(screen.getByText("Pause"));
    await waitFor(() => {
      expect(onSubmitCommand).toHaveBeenCalledWith(
        expect.any(String),
        expect.anything(),
        undefined,
        {
          type: "user_message",
          content: formatAgentActionCommand(
            "pause",
            "T-29",
            DEFAULT_PAUSE_REASON,
          ),
        },
        true,
      );
    });

    fireEvent.click(screen.getByText("Close"));
    fireEvent.click(screen.getByText("Diff"));
    await waitFor(() => {
      expect(onSubmitCommand).toHaveBeenCalledWith(
        expect.any(String),
        expect.anything(),
        undefined,
        {
          type: "user_message",
          content: formatAgentActionCommand("diff", "T-29"),
        },
        true,
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
        expect.any(String),
        expect.anything(),
        undefined,
        {
          type: "user_message",
          content: formatAgentActionCommand(
            "steer",
            "T-29",
            "Please add the renderer tests",
          ),
        },
        true,
      );
    });

    fireEvent.click(screen.getByText("Close"));
    fireEvent.click(screen.getByText("Cancel"));
    expect(onSubmitCommand).toHaveBeenCalledTimes(3);
    fireEvent.click(screen.getByText("Confirm cancel"));
    await waitFor(() => {
      expect(onSubmitCommand).toHaveBeenCalledWith(
        expect.any(String),
        expect.anything(),
        undefined,
        {
          type: "user_message",
          content: formatAgentActionCommand(
            "cancel",
            "T-29",
            DEFAULT_CANCEL_REASON,
          ),
        },
        true,
      );
    });
    expect(outer).toHaveAttribute("aria-expanded", "true");
  });
});

function itemOf(toggle: HTMLElement): HTMLElement {
  const section = toggle.closest("section");
  if (!section) throw new Error("expected transcript section");
  return section;
}

describe("AgentPulseView transcript route", () => {
  it("starts closed with the card title and reported state, revealing controls when opened", async () => {
    const { user } = renderPulse(PULSE_OUTPUT);
    const outer = screen.getByTestId("transcript-toggle");
    expect(outer).toHaveAttribute("aria-expanded", "false");
    expect(itemOf(outer)).toHaveAttribute("data-status", "done");
    expect(within(outer).getByTestId("transcript-verb")).toHaveTextContent(
      "Pulse",
    );
    expect(within(outer).getByTestId("transcript-target")).toHaveTextContent(
      "check-agents redesign",
    );
    expect(outer).not.toHaveTextContent("T-29");
    expect(screen.queryByTestId("transcript-status-pill")).toBeNull();
    expect(itemOf(outer).querySelector(".rf-spin")).toBeNull();
    expect(within(outer).getByTestId("transcript-meta")).toHaveTextContent(
      "generating response",
    );
    expect(
      screen.queryByRole("button", { name: "Pause" }),
    ).not.toBeInTheDocument();
    await user.click(outer);
    const facts = screen.getByTestId("transcript-facts");
    expect(facts.querySelectorAll("dt")).toHaveLength(3);
    expect(
      within(facts).getByText("🔄 generating response"),
    ).toBeInTheDocument();
    expect(within(facts).getByText("3m ago")).toBeInTheDocument();
    const pause = screen.getByRole("button", { name: "Pause" });
    expect(pause.closest("[inert]")).toBeNull();
    expect(
      screen.getByText("Adding sticky alerts logic..."),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Raw" }));
    const raw = screen.getByTestId("transcript-raw");
    const blocks = Array.from(raw.querySelectorAll("pre")).map(
      (pre) => pre.textContent,
    );
    expect(blocks).toEqual([
      JSON.stringify({ card_id: "T-29" }, null, 2),
      PULSE_OUTPUT,
    ]);
  });

  it.each([
    ["💤 idle", "done", null],
    ["⏸️ paused", "done", null],
    ["⏳ waiting for user input", "done", null],
    ["✅ completed", "done", null],
    ["❌ error", "failed", "Failed"],
    ["unknown", "done", null],
  ])(
    "reports the snapshot state %s in meta; only alerts become the row status",
    (state, status, pillText) => {
      renderPulse(PULSE_OUTPUT.replace("🔄 generating response", state));
      const outer = screen.getByTestId("transcript-toggle");
      expect(outer).toHaveAttribute("aria-expanded", "false");
      expect(itemOf(outer)).toHaveAttribute("data-status", status);
      const pill = screen.queryByTestId("transcript-status-pill");
      if (pillText === null) expect(pill).not.toBeInTheDocument();
      else expect(pill).toHaveTextContent(pillText);
      expect(outer).not.toHaveTextContent(/progress|verified success/iu);
    },
  );

  it.each([
    [null, false, false, "unknown", "No result"],
    [null, false, true, "running", "Running"],
    ["not a report", false, false, "unknown", "Unknown"],
    ['cancel_agent(card_id="T-29")', true, false, "failed", "Failed"],
  ])(
    "keeps absent/malformed/failed transport honest: %s",
    (content, failed, waiting, status, pillText) => {
      const send = vi
        .spyOn(chatCommands, "sendChatCommand")
        .mockResolvedValue(undefined);
      renderPulse(content, failed, waiting);
      const outer = screen.getByTestId("transcript-toggle");
      expect(outer).toHaveAttribute("aria-expanded", "false");
      expect(itemOf(outer)).toHaveAttribute("data-status", status);
      expect(
        within(outer).queryByTestId("transcript-target"),
      ).not.toBeInTheDocument();
      expect(screen.getByTestId("transcript-status-pill")).toHaveTextContent(
        pillText,
      );
      expect(send).not.toHaveBeenCalled();
    },
  );

  it("falls back to the card id when the pulse has no title", () => {
    renderPulse("# Agent Pulse: T-29");
    const outer = screen.getByTestId("transcript-toggle");
    expect(within(outer).getByTestId("transcript-target")).toHaveTextContent(
      "T-29",
    );
    expect(itemOf(outer)).toHaveAttribute("data-status", "done");
  });

  it("shows a failed request in the Error section without pulse controls", async () => {
    const { user } = renderPulse("pulse failed sentinel", true);
    await user.click(screen.getByTestId("transcript-toggle"));
    expect(screen.getByRole("alert")).toHaveTextContent(
      "pulse failed sentinel",
    );
    expect(
      screen.queryByRole("button", { name: "Pause" }),
    ).not.toBeInTheDocument();
  });

  it("keeps stored disclosure choices independent across status changes and remounts", () => {
    const preferences = new Map([["agent_pulse:pulse-call", true]]);
    const collapseStore = {
      get: (key: string) => preferences.get(key),
      set: (key: string, value: boolean) => {
        preferences.set(key, value);
      },
    };
    const chat = createDefaultChatState();
    const threadId = chat.current_thread_id;
    chat.threads[threadId].thread.messages = [pulseMessage(PULSE_OUTPUT)];
    const view = () => (
      <CollapsibleStoreProvider value={collapseStore}>
        <AgentPulseView toolCall={TOOL_CALL} />
        <AgentPulseView toolCall={{ ...TOOL_CALL, id: "second-pulse-call" }} />
      </CollapsibleStoreProvider>
    );
    const { store, unmount } = render(view(), { preloadedState: { chat } });
    const [outer, sibling] = screen.getAllByTestId("transcript-toggle");
    expect(outer).toHaveAttribute("aria-expanded", "true");
    act(() => {
      store.dispatch(
        applyChatEvent({
          chat_id: threadId,
          seq: "1",
          type: "message_added",
          index: 0,
          message: pulseMessage(
            PULSE_OUTPUT.replace("🔄 generating response", "⏸️ paused"),
          ),
        }),
      );
    });
    expect(outer).toHaveAttribute("aria-expanded", "true");
    expect(
      within(itemOf(outer)).getByTestId("transcript-meta"),
    ).toHaveTextContent("paused");
    expect(sibling).toHaveAttribute("aria-expanded", "false");
    fireEvent.click(outer);
    act(() => {
      store.dispatch(
        applyChatEvent({
          chat_id: threadId,
          seq: "2",
          type: "message_added",
          index: 0,
          message: pulseMessage("failed", true),
        }),
      );
    });
    expect(outer).toHaveAttribute("aria-expanded", "false");
    unmount();
    render(view(), { store });
    expect(screen.getAllByTestId("transcript-toggle")[0]).toHaveAttribute(
      "aria-expanded",
      "false",
    );
    expect(preferences.get("agent_pulse:pulse-call")).toBe(false);
  });

  it("preserves disabled controls", () => {
    render(<AgentPulseContent report={parsedReport()} actionsDisabled />);
    for (const name of ["Steer", "Pause", "Cancel", "Diff"]) {
      expect(screen.getByRole("button", { name })).toBeDisabled();
    }
  });
});
