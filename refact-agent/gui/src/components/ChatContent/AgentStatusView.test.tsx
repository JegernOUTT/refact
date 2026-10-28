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
  toolCall = TOOL_CALL,
) {
  const chat = createDefaultChatState();
  const threadId = chat.current_thread_id;
  chat.threads[threadId].thread.messages =
    content === null ? [] : [statusMessage(content, toolFailed)];
  chat.threads[threadId].waiting_for_response = waiting;
  return {
    ...render(<AgentStatusView toolCall={toolCall} />, {
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
P1 ⏸️  T-5   paused-card            | paused     | 10m ago | approval
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

    expect(report.alerts).toEqual({ stuck: 1, failed: 1, approval: 1 });
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
    const outer = screen.getByTestId("transcript-toggle");
    fireEvent.click(outer);
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

function itemOf(toggle: HTMLElement): HTMLElement {
  const section = toggle.closest("section");
  if (!section) throw new Error("expected transcript section");
  return section;
}

function rawTexts(): string[] {
  const raw = screen.getByTestId("transcript-raw");
  return Array.from(raw.querySelectorAll("pre")).map(
    (pre) => pre.textContent ?? "",
  );
}

describe("AgentStatusView transcript route", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("keeps queued, parked and planner-paused A/B rows distinct without inventing approval", async () => {
    const send = vi
      .spyOn(chatCommands, "sendChatCommand")
      .mockResolvedValue(undefined);
    const output = `⚠️  Alerts: 0 stuck, 0 failed, 1 needing approval, 1 needing attention

Delta since seq 42

P1 ⏳  T-AB/a queued-variant | queued | waiting for capacity
P1 ⏸️  T-AB/b parked-variant | parked | capacity released | ⚠ needs attention: awaiting dependency (since 2026-05-21T23:30:00Z, ×3)
P1 ⏸️  T-PAUSE manual-pause | paused by planner-test since 2026-05-21T23:55:00Z
showing 3 of 8; pass offset=3 for next page
`;
    const { user } = renderStatus(output);
    const outer = screen.getByTestId("transcript-toggle");
    expect(within(outer).getByTestId("transcript-verb")).toHaveTextContent(
      "Agents",
    );
    expect(within(outer).getByTestId("transcript-target")).toHaveTextContent(
      "3 cards",
    );
    expect(
      screen.queryByTestId("transcript-status-pill"),
    ).not.toBeInTheDocument();
    await user.click(outer);
    const facts = screen.getByTestId("transcript-facts");
    expect(
      within(facts).getByText("0 stuck · 0 failed · 0 approval"),
    ).toBeInTheDocument();
    expect(
      within(facts).getByText("showing 3 of 8; pass offset=3 for next page"),
    ).toBeInTheDocument();
    const table = screen.getByRole("table");
    const variantA = within(table).getByText("T-AB/a").closest("article");
    const variantB = within(table).getByText("T-AB/b").closest("article");
    if (!variantA || !variantB) throw new Error("expected both variant cards");
    expect(within(variantA).getByText("queued")).toBeInTheDocument();
    expect(within(variantA).getByText("Queued")).toBeInTheDocument();
    expect(within(variantB).getByText("parked")).toBeInTheDocument();
    expect(within(variantB).getByText("Parked")).toBeInTheDocument();
    expect(within(variantA).getByText("unknown")).toBeInTheDocument();
    expect(within(variantB).getByText("unknown")).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Queued 1" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Parked 1" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Paused 1" })).toBeInTheDocument();
    expect(screen.getByLabelText("Steer T-AB/a")).toBeDisabled();
    expect(screen.getByLabelText("Cancel agent T-AB/b")).toBeDisabled();
    expect(
      screen.getByText(
        /Variant steering and cancellation are unavailable; inspection targets T-AB\/b only\./u,
      ),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Raw" }));
    expect(rawTexts()).toEqual(["{}", output]);
    expect(send).not.toHaveBeenCalled();
    await user.click(screen.getByLabelText("View pulse T-AB/a"));
    await waitFor(() =>
      expect(send).toHaveBeenCalledWith(
        expect.any(String),
        expect.anything(),
        undefined,
        { type: "user_message", content: 'agent_pulse(card_id="T-AB/a")' },
        true,
      ),
    );
    await user.click(screen.getByRole("button", { name: "Close" }));
    await user.click(screen.getByLabelText("View diff T-AB/b"));
    await waitFor(() =>
      expect(send).toHaveBeenLastCalledWith(
        expect.any(String),
        expect.anything(),
        undefined,
        { type: "user_message", content: 'agent_diff(card_id="T-AB/b")' },
        true,
      ),
    );
  });

  it("preserves JSON unknown and cancelled states, exact variant IDs and explicit approval evidence", async () => {
    const output = JSON.stringify({
      agents: [
        {
          card_id: "T-AB",
          variant_key: "a",
          title: "Unknown variant",
          status: "future_state",
        },
        { card_id: "T-CANCEL", title: "Cancelled worker", status: "cancelled" },
        { card_id: "T-PAUSE", title: "Planner pause", status: "paused" },
        {
          card_id: "T-APPROVE",
          title: "Explicit approval",
          status: "waiting_for_approval",
          approval_required: true,
        },
      ],
      alerts: { paused: 4 },
      omitted: "JSON omission sentinel",
    });
    const { user } = renderStatus(output);
    const outer = screen.getByTestId("transcript-toggle");
    expect(within(outer).getByTestId("transcript-target")).toHaveTextContent(
      "4 cards",
    );
    expect(itemOf(outer)).toHaveAttribute("data-status", "needs_you");
    expect(screen.getByTestId("transcript-status-pill")).toHaveTextContent(
      "1 need approval",
    );
    expect(outer).toHaveAttribute("aria-expanded", "true");
    expect(
      within(screen.getByTestId("transcript-facts")).getByText(
        "0 stuck · 0 failed · 1 approval",
        { selector: "dd" },
      ),
    ).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Unknown 1" })).toBeInTheDocument();
    expect(
      screen.getByRole("tab", { name: "Cancelled 1" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Paused 2" })).toBeInTheDocument();
    const unknown = within(screen.getByRole("table"))
      .getByText("T-AB/a")
      .closest("article");
    if (!unknown) throw new Error("expected unknown variant card");
    expect(within(unknown).getByText("future_state")).toBeInTheDocument();
    expect(within(unknown).getByText("Unknown")).toBeInTheDocument();
    expect(within(unknown).queryByText("Running")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Raw" }));
    expect(rawTexts()[1]).toContain("JSON omission sentinel");
  });

  it("recovers the complete detail body and arguments from Raw when the roster is not parsed", async () => {
    const args = '{"format":"detail","card_ids":["T-AB/a"],"offset":2}';
    const output = `⚠️  Alerts: 0 stuck, 0 failed, 0 needing approval, 1 needing attention

# Agent Status Detail

Showing 1 of 4 agents.

### ⏸️ Detailed worker [A/B A] (T-AB/a)
**Status:** Parked / waiting without execution capacity | **Column:** doing | **Priority:** P1 | **Chat:** \`agent-chat-a\`
⚠ needs attention: external dependency (since 2026-05-21T23:30:00Z, ×3)
**A/B variant:** A
**Last tool:** \`cat\`

**Last Update:** ${"x".repeat(5_000)}
DETAIL_SENTINEL

---

showing 1 of 4; pass offset=3 for next page
`;
    const { user } = renderStatus(output, false, false, {
      ...TOOL_CALL,
      function: { name: "check_agents", arguments: args },
    });
    const outer = screen.getByTestId("transcript-toggle");
    expect(
      within(outer).queryByTestId("transcript-target"),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByTestId("transcript-status-pill"),
    ).not.toBeInTheDocument();
    expect(screen.queryByTestId("transcript-raw")).not.toBeInTheDocument();
    await user.click(outer);
    expect(screen.queryByRole("table")).not.toBeInTheDocument();
    expect(screen.queryByTestId("transcript-facts")).not.toBeInTheDocument();
    expect(
      screen.queryByText("No agents match the selected filters."),
    ).not.toBeInTheDocument();
    expect(screen.queryByText(/DETAIL_SENTINEL/u)).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Raw" }));
    const [rawArgs, rawResponse] = rawTexts();
    expect(JSON.parse(rawArgs)).toEqual(JSON.parse(args));
    expect(rawResponse).toBe(output);
    expect(screen.getAllByText(/DETAIL_SENTINEL/u)).toHaveLength(1);
  });

  it("keeps the current summary and attention text instead of substituting an empty roster", async () => {
    const output = `⚠️  Alerts: 0 stuck, 0 failed, 1 needing approval, 1 needing attention

Total: 3 | Running: 0 | Stuck: 0 | Failed: 0 | Done: 0 | Paused: 1 | Queued: 1 | Parked: 1
By priority: P1=3
T-PAUSE: paused by planner-test since 2026-05-21T23:55:00Z
T-AB: ⚠ needs attention: summary attention sentinel (since 2026-05-21T23:30:00Z, ×3)
`;
    const { user } = renderStatus(output);
    await user.click(screen.getByTestId("transcript-toggle"));
    expect(screen.queryByRole("table")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Raw" }));
    expect(rawTexts()[1]).toBe(output);
  });

  it("makes malformed bodies inspectable and distinguishes them from a legitimate empty roster", async () => {
    const { user, unmount } = renderStatus(
      '{"agents":[{}],"note":"malformed sentinel"}',
    );
    await user.click(screen.getByTestId("transcript-toggle"));
    expect(screen.queryByRole("table")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Raw" }));
    expect(rawTexts()[1]).toContain("malformed sentinel");
    unmount();
    renderStatus('{"agents":[]}');
    const outer = screen.getByTestId("transcript-toggle");
    expect(within(outer).getByTestId("transcript-target")).toHaveTextContent(
      "0 cards",
    );
    await user.click(outer);
    expect(
      screen.getByText("No agent rows returned in this response."),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Raw" }));
    expect(rawTexts()[1]).toBe(JSON.stringify({ agents: [] }, null, 2));
  });

  it("starts closed with observed counts and visible alert state", async () => {
    const { user } = renderStatus(
      COMPACT_OUTPUT.replace(
        "showing 5 of 5; no more pages",
        "showing 5 of 50; pass offset=5 for next page",
      ),
    );
    const outer = screen.getByTestId("transcript-toggle");
    expect(outer).toHaveAttribute("aria-expanded", "false");
    expect(itemOf(outer)).toHaveAttribute("data-status", "failed");
    expect(within(outer).getByTestId("transcript-target")).toHaveTextContent(
      "5 cards",
    );
    expect(screen.getByTestId("transcript-status-pill")).toHaveTextContent(
      "1 failed",
    );
    expect(screen.queryByLabelText("View pulse T-1")).not.toBeInTheDocument();
    await user.click(outer);
    const facts = screen.getByTestId("transcript-facts");
    expect(facts.querySelectorAll("dt")).toHaveLength(3);
    expect(
      within(facts).getByText("showing 5 of 50; pass offset=5 for next page"),
    ).toBeInTheDocument();
    const pulse = screen.getByLabelText("View pulse T-1");
    expect(pulse.closest("[inert]")).toBeNull();
    expect(pulse).not.toHaveAttribute("tabindex", "-1");
  });

  it.each([
    [
      "1 need approval",
      "P1 ⏸️  T-5   paused-card            | paused     | 10m ago | approval",
    ],
    [
      "1 stuck",
      "P1 🔴  T-2   fix-tests              | STUCK 18m   | needs attention",
    ],
  ])("labels attention alerts as %s and opens by default", (label, row) => {
    renderStatus(
      `P0 🔄  T-1   implement-render       | generating |  3m ago | last: cat\n${row}\n`,
    );
    const outer = screen.getByTestId("transcript-toggle");
    expect(itemOf(outer)).toHaveAttribute("data-status", "needs_you");
    expect(screen.getByTestId("transcript-status-pill")).toHaveTextContent(
      label,
    );
    expect(outer).toHaveAttribute("aria-expanded", "true");
  });

  it.each([
    [null, false, false, null, "unknown", "Unknown"],
    [null, false, true, null, "running", "Running"],
    ["not a report", false, false, null, "done", null],
    ['cancel_agent(card_id="T-1")', true, false, null, "failed", "Failed"],
    ['{"agents":[]}', false, false, "0 cards", "done", null],
    ['{"agents":[{}]}', false, false, null, "done", null],
    ['{"agents":[{"card_id":"T-1"}]}', false, false, "1 cards", "done", null],
    [
      "⚠️ Alerts: 0 stuck, 0 failed, 0 needing approval",
      false,
      false,
      null,
      "done",
      null,
    ],
  ])(
    "keeps empty, missing and malformed results distinct: %s",
    (content, failed, waiting, target, status, pillText) => {
      const send = vi
        .spyOn(chatCommands, "sendChatCommand")
        .mockResolvedValue(undefined);
      renderStatus(content, failed, waiting);
      const outer = screen.getByTestId("transcript-toggle");
      expect(outer).toHaveAttribute("aria-expanded", "false");
      expect(itemOf(outer)).toHaveAttribute("data-status", status);
      const targetNode = within(outer).queryByTestId("transcript-target");
      if (target === null) expect(targetNode).not.toBeInTheDocument();
      else expect(targetNode).toHaveTextContent(target);
      const pill = screen.queryByTestId("transcript-status-pill");
      if (pillText === null) expect(pill).not.toBeInTheDocument();
      else expect(pill).toHaveTextContent(pillText);
      expect(send).not.toHaveBeenCalled();
    },
  );

  it("shows a failed request in the Error section", async () => {
    const { user } = renderStatus("request failed sentinel", true);
    await user.click(screen.getByTestId("transcript-toggle"));
    expect(screen.getByRole("alert")).toHaveTextContent(
      "request failed sentinel",
    );
    expect(screen.queryByRole("table")).not.toBeInTheDocument();
  });

  it("honors stored choices and independent rows after attention updates and remount", () => {
    const preferences = new Map([["agent_status:status-call", true]]);
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
    const [outer, sibling] = screen.getAllByTestId("transcript-toggle");
    expect(outer).toHaveAttribute("aria-expanded", "true");
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
      screen.getByText("1 stuck, 2 failed, 1 needing approval"),
    ).toBeInTheDocument();
    expect(
      within(itemOf(outer)).getByTestId("transcript-status-pill"),
    ).toHaveTextContent("2 failed");
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
    expect(screen.getAllByTestId("transcript-toggle")[0]).toHaveAttribute(
      "aria-expanded",
      "false",
    );
    expect(preferences.get("agent_status:status-call")).toBe(false);
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
