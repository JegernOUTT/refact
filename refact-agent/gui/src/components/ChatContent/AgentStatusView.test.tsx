import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { act as rtlAct } from "@testing-library/react";
import {
  fireEvent,
  createDefaultChatState,
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
  filterAgentStatusRows,
  formatAgentActionCommand,
  parseAgentStatusOutput,
  type AgentStatusReport,
} from "./AgentStatusModel";
import { AgentStatusContent, AgentStatusView } from "./AgentStatusView";

const act = rtlAct as unknown as (callback: () => void) => void;

const TOOL_CALL: ToolCall = {
  id: "status-call",
  index: 0,
  function: { name: "check_agents", arguments: "{}" },
};

function statusMessage(content: string, tool_failed = false): ToolMessage {
  return {
    role: "tool",
    message_id: "status-result",
    tool_call_id: "status-call",
    content,
    tool_failed,
  };
}

function renderStatus(
  content: string | null,
  toolFailed = false,
  waiting = false,
) {
  const chat = createDefaultChatState();
  const threadId = chat.current_thread_id;
  chat.threads[threadId].thread.messages =
    content === null ? [] : [statusMessage(content, toolFailed)];
  chat.threads[threadId].waiting_for_response = waiting;
  return {
    ...render(<AgentStatusView toolCall={TOOL_CALL} />, {
      preloadedState: { chat },
    }),
    threadId,
  };
}

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
    const onSubmitCommand = vi
      .spyOn(chatCommands, "sendChatCommand")
      .mockResolvedValue(undefined);
    renderStatus(COMPACT_OUTPUT);
    const outer = screen.getByRole("button", {
      name: "5 agent rows shown Failed / approval",
    });
    fireEvent.click(outer);
    fireEvent.click(screen.getByRole("button", { name: "More details" }));
    expect(onSubmitCommand).not.toHaveBeenCalled();

    fireEvent.click(screen.getByLabelText("View pulse T-1"));
    await waitFor(() => {
      expect(onSubmitCommand).toHaveBeenCalledWith(
        expect.any(String),
        expect.anything(),
        undefined,
        {
          type: "user_message",
          content: formatAgentActionCommand("pulse", "T-1"),
        },
        true,
      );
    });

    fireEvent.click(screen.getByText("Close"));
    fireEvent.click(screen.getByLabelText("View diff T-1"));
    await waitFor(() => {
      expect(onSubmitCommand).toHaveBeenCalledWith(
        expect.any(String),
        expect.anything(),
        undefined,
        {
          type: "user_message",
          content: formatAgentActionCommand("diff", "T-1"),
        },
        true,
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
        expect.any(String),
        expect.anything(),
        undefined,
        {
          type: "user_message",
          content: formatAgentActionCommand(
            "steer",
            "T-1",
            "Please check the failing test",
          ),
        },
        true,
      );
    });

    fireEvent.click(screen.getByText("Close"));
    fireEvent.click(screen.getByLabelText("Cancel agent T-1"));
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
            "T-1",
            "Cancelled from agent status view.",
          ),
        },
        true,
      );
    });
    expect(outer).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByLabelText("Toggle details T-1")).toHaveAttribute(
      "aria-expanded",
      "false",
    );
  });
});

describe("AgentStatusView compact route", () => {
  it("starts closed with observed counts and visible alert state, with inert secondary controls", async () => {
    const { container, user } = renderStatus(
      COMPACT_OUTPUT.replace(
        "showing 5 of 5; no more pages",
        "showing 5 of 50; pass offset=5 for next page",
      ),
    );
    const outer = screen.getByRole("button", {
      name: "5 agent rows shown Failed / approval",
    });
    expect(outer).toHaveAttribute("aria-expanded", "false");
    expect(outer.closest("section")).toHaveAttribute("data-density", "compact");
    expect(screen.queryByLabelText("View pulse T-1")).not.toBeInTheDocument();
    await user.click(outer);
    expect(container.querySelectorAll("dt")).toHaveLength(3);
    expect(
      screen.getByText("Returned rows only; total unknown"),
    ).toBeInTheDocument();
    const details = screen.getByRole("button", { name: "More details" });
    const pulse = screen.getByLabelText("View pulse T-1");
    expect(details).toHaveAttribute("aria-expanded", "false");
    expect(pulse.closest("[inert]")).not.toBeNull();
    details.focus();
    await user.tab();
    expect(pulse).not.toHaveFocus();
    await user.click(details);
    expect(pulse.closest("[inert]")).toBeNull();
    expect(pulse).not.toHaveAttribute("tabindex", "-1");
  });

  it.each([
    [null, false, false, "Check agent status Status unknown", "idle"],
    [null, false, true, "Check agent status Checking", "running"],
    ["not a report", false, false, "Check agent status Status unknown", "idle"],
    [
      'cancel_agent(card_id="T-1")',
      true,
      false,
      "Check agent status Request failed",
      "error",
    ],
    [
      '{"agents":[]}',
      false,
      false,
      "0 agent rows shown Snapshot received",
      "idle",
    ],
    [
      '{"agents":[{}]}',
      false,
      false,
      "Check agent status Status unknown",
      "idle",
    ],
    [
      '{"agents":[{"card_id":"T-1"}]}',
      false,
      false,
      "1 agent rows shown Snapshot received",
      "idle",
    ],
    [
      "⚠️ Alerts: 0 stuck, 0 failed, 0 needing approval",
      false,
      false,
      "Check agent status Snapshot received",
      "idle",
    ],
  ])(
    "keeps empty, missing and malformed results distinct: %s",
    (content, failed, waiting, name, status) => {
      const send = vi
        .spyOn(chatCommands, "sendChatCommand")
        .mockResolvedValue(undefined);
      renderStatus(content, failed, waiting);
      const outer = screen.getByRole("button", { name });
      expect(outer).toHaveAttribute("aria-expanded", "false");
      expect(outer.closest("section")).toHaveAttribute("data-status", status);
      expect(send).not.toHaveBeenCalled();
    },
  );

  it("honors stored choices and independent rows after attention updates and remount", () => {
    const preferences = new Map([["tc:status-call", true]]);
    const collapseStore = {
      get: (key: string) => preferences.get(key),
      set: (key: string, value: boolean) => {
        preferences.set(key, value);
      },
    };
    const chat = createDefaultChatState();
    const threadId = chat.current_thread_id;
    chat.threads[threadId].thread.messages = [statusMessage(COMPACT_OUTPUT)];
    const view = () => (
      <CollapsibleStoreProvider value={collapseStore}>
        <AgentStatusView toolCall={TOOL_CALL} />
        <AgentStatusView
          toolCall={{ ...TOOL_CALL, id: "second-status-call" }}
        />
      </CollapsibleStoreProvider>
    );
    const { store, unmount } = render(view(), { preloadedState: { chat } });
    const outer = screen.getByRole("button", {
      name: "5 agent rows shown Failed / approval",
    });
    const sibling = screen.getByRole("button", {
      name: "Check agent status Status unknown",
    });
    expect(outer).toHaveAttribute("aria-expanded", "true");
    fireEvent.click(screen.getByRole("button", { name: "More details" }));
    fireEvent.click(screen.getByLabelText("Toggle details T-1"));
    act(() => {
      store.dispatch(
        applyChatEvent({
          chat_id: threadId,
          seq: "1",
          type: "message_added",
          index: 0,
          message: statusMessage(
            COMPACT_OUTPUT.replace("1 failed", "2 failed"),
          ),
        }),
      );
    });
    expect(outer).toHaveAttribute("aria-expanded", "true");
    expect(
      screen.getAllByText("1 stuck, 2 failed, 1 needing approval"),
    ).toHaveLength(2);
    expect(screen.getByLabelText("Toggle details T-1")).toHaveAttribute(
      "aria-expanded",
      "true",
    );
    expect(sibling).toHaveAttribute("aria-expanded", "false");
    fireEvent.click(outer);
    act(() => {
      store.dispatch(
        applyChatEvent({
          chat_id: threadId,
          seq: "2",
          type: "message_added",
          index: 0,
          message: statusMessage("request failed", true),
        }),
      );
    });
    expect(outer).toHaveAttribute("aria-expanded", "false");
    unmount();
    render(view(), { store });
    expect(
      screen.getByRole("button", { name: "Check agent status Request failed" }),
    ).toHaveAttribute("aria-expanded", "false");
    expect(preferences.get("tc:status-call")).toBe(false);
  });

  it("preserves disabled content actions", () => {
    render(<AgentStatusContent report={parsedReport()} actionsDisabled />);
    for (const name of [
      "View pulse T-1",
      "View diff T-1",
      "Steer T-1",
      "Cancel agent T-1",
    ]) {
      expect(screen.getByLabelText(name)).toBeDisabled();
    }
  });
});
