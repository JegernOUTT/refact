import { afterEach, describe, expect, it, vi } from "vitest";
import { act as rtlAct } from "@testing-library/react";
import {
  createDefaultChatState,
  fireEvent,
  render,
  screen,
  waitFor,
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
    content === null ? [] : [pulseMessage(content, toolFailed)];
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
    const onSubmitCommand = vi
      .spyOn(chatCommands, "sendChatCommand")
      .mockResolvedValue(undefined);
    renderPulse(PULSE_OUTPUT);
    const outer = screen.getByRole("button", {
      name: "Inspect check-agents redesign Agent running",
    });
    fireEvent.click(outer);
    const details = screen.getByRole("button", { name: "More details" });
    fireEvent.click(details);
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
    expect(details).toHaveAttribute("aria-expanded", "true");
  });
});

describe("AgentPulseView compact route", () => {
  it("starts closed with a title and explicit agent state, with hidden controls outside the tab order", async () => {
    const { user, container } = renderPulse(PULSE_OUTPUT);
    const outer = screen.getByRole("button", {
      name: "Inspect check-agents redesign Agent running",
    });
    expect(outer).toHaveAttribute("aria-expanded", "false");
    expect(outer.closest("section")).toHaveAttribute("data-density", "compact");
    expect(outer.closest("section")).toHaveAttribute("data-status", "idle");
    expect(outer).not.toHaveTextContent("T-29");
    expect(
      screen.queryByRole("button", { name: "Pause" }),
    ).not.toBeInTheDocument();
    await user.click(outer);
    expect(container.querySelectorAll("dt")).toHaveLength(3);
    const details = screen.getByRole("button", { name: "More details" });
    const pause = screen.getByRole("button", { name: "Pause" });
    expect(pause.closest("[inert]")).not.toBeNull();
    details.focus();
    await user.tab();
    expect(pause).not.toHaveFocus();
    await user.click(details);
    expect(pause.closest("[inert]")).toBeNull();
    expect(
      screen.getByText("Adding sticky alerts logic..."),
    ).toBeInTheDocument();
  });

  it.each([
    ["💤 idle", "Agent idle"],
    ["⏸️ paused", "Agent paused"],
    ["⏳ waiting for user input", "Agent waiting"],
    ["✅ completed", "Done · unverified"],
    ["❌ error", "Agent failed"],
    ["❓ last known: done", "State unknown"],
    ["not completed; failed?", "State unknown"],
    ["unknown", "State unknown"],
  ])(
    "uses only explicit reported state %s, never activity as progress",
    (state, label) => {
      renderPulse(PULSE_OUTPUT.replace("🔄 generating response", state));
      const outer = screen.getByRole("button", {
        name: `Inspect check-agents redesign ${label}`,
      });
      expect(outer).toHaveAttribute("aria-expanded", "false");
      expect(outer.closest("section")).toHaveAttribute("data-status", "idle");
      expect(outer).not.toHaveTextContent(/progress|verified success/iu);
    },
  );

  it.each([
    [null, false, false, "State unknown", "idle"],
    [null, false, true, "Checking pulse", "running"],
    ["not a report", false, false, "State unknown", "idle"],
    ['cancel_agent(card_id="T-29")', true, false, "Request failed", "error"],
    ["# Agent Pulse: T-29", false, false, "State unknown", "idle"],
  ])(
    "keeps absent/malformed/failed transport honest: %s",
    (content, failed, waiting, label, status) => {
      const send = vi
        .spyOn(chatCommands, "sendChatCommand")
        .mockResolvedValue(undefined);
      renderPulse(content, failed, waiting);
      const outer = screen.getByRole("button", {
        name: `Inspect agent activity ${label}`,
      });
      expect(outer).toHaveAttribute("aria-expanded", "false");
      expect(outer.closest("section")).toHaveAttribute("data-status", status);
      expect(send).not.toHaveBeenCalled();
    },
  );

  it("keeps stored disclosure choices independent across status changes and remounts", () => {
    const preferences = new Map([["tc:pulse-call", true]]);
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
    const outer = screen.getByRole("button", {
      name: "Inspect check-agents redesign Agent running",
    });
    const sibling = screen.getByRole("button", {
      name: "Inspect agent activity State unknown",
    });
    const details = screen.getByRole("button", { name: "More details" });
    fireEvent.click(details);
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
    expect(outer).toHaveTextContent("Agent paused");
    expect(details).toHaveAttribute("aria-expanded", "true");
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
    expect(
      screen.getByRole("button", {
        name: "Inspect agent activity Request failed",
      }),
    ).toHaveAttribute("aria-expanded", "false");
  });

  it("preserves disabled controls", () => {
    render(<AgentPulseContent report={parsedReport()} actionsDisabled />);
    for (const name of ["Steer", "Pause", "Cancel", "Diff"]) {
      expect(screen.getByRole("button", { name })).toBeDisabled();
    }
  });
});
