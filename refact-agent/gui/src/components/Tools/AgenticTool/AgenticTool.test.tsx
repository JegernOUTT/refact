import type { ReactElement } from "react";
import { describe, expect, test, vi } from "vitest";
import { fireEvent, within } from "@testing-library/react";
import { act } from "react-dom/test-utils";
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
import { applyChatEvent } from "../../../features/Chat/Thread/actions";
import { ToolContent } from "../../ChatContent/ToolsContent";
import { CollapsibleStoreProvider } from "../../ChatContent/useStoredOpen";
import { ChatScrollAnchorContext } from "../../ChatContent/useChatScrollAnchor";
import {
  AGENTIC_CARD_LABELS,
  AGENTIC_TOOL_NAMES,
  BOARD_RUNTIME_TOOL_NAMES,
  DEDICATED_TOOL_CARD_NAMES,
  LOCAL_COLLABORATION_TOOL_NAMES,
  agenticCardLabel,
  hasDedicatedToolCard,
  isAgenticToolName,
  lookupAgenticFamily,
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

function renderTools(
  messages: ChatMessages,
  calls: ToolCall[],
  wrap: (content: ReactElement) => ReactElement = (content) => content,
) {
  const chat = createDefaultChatState();
  const runtime = chat.threads[chat.current_thread_id];
  runtime.thread.id = CHAT_ID;
  runtime.thread.messages = messages;
  runtime.streaming = false;
  runtime.waiting_for_response = false;
  chat.current_thread_id = CHAT_ID;
  chat.open_thread_ids = [CHAT_ID];
  chat.threads = { [CHAT_ID]: runtime };

  return render(wrap(<ToolContent toolCalls={calls} threadId={CHAT_ID} />), {
    preloadedState: { chat },
  });
}

function openFullFamily() {
  fireEvent.click(screen.getAllByRole("button")[0]);
  fireEvent.click(screen.getByRole("button", { name: "More details" }));
}

const INVENTORY = Array.from(
  new Set([...AGENTIC_TOOL_NAMES, ...BOARD_RUNTIME_TOOL_NAMES]),
).filter((name) => isAgenticToolName(name));

describe("AgenticTool card inventory", () => {
  test("every agentic family and tool name owns a dedicated card", () => {
    const families = AGENTIC_TOOL_NAMES.map(
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

  test.each(INVENTORY.map((name) => [name] as const))(
    "%s renders a dedicated card and never the generic fallback",
    (name) => {
      const label = agenticCardLabel(name);
      expect(label, name).not.toBeNull();
      const { unmount } = renderTools(
        [toolResult(name, "# Outcome\n\n- Status: running\n\nrecorded")],
        [
          toolCall(name, {
            agent_id: "abc",
            card_id: "T-1",
            task_id: "task-1",
            to: "peer-1",
            text: "hello",
            status_line: "hello",
          }),
        ],
      );
      expect(screen.getByTestId("agentic-tool")).toBeInTheDocument();
      expect(screen.queryByTestId("generic-tool")).toBeNull();
      expect(screen.queryByTestId("agentic-tool-view")).toBeNull();
      const dedicated =
        screen.queryAllByLabelText(label ?? "").length +
        screen.queryAllByRole("alert").length +
        screen.queryAllByRole("status").length;
      expect(dedicated, name).toBeGreaterThan(0);
      unmount();
    },
  );
});

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
    openFullFamily();
    const response = within(screen.getByRole("region", { name: "Response" }));
    const shown = screen.getByTestId("agentic-tool-body");
    expect(shown.textContent?.length).toBe(PAGE_CHARS);
    expect(response.getByRole("status")).toHaveTextContent("Section 1 of 3");
    const next = response.getByRole("button", { name: "Next section" });
    fireEvent.click(next);
    expect(response.getByRole("status")).toHaveTextContent("Section 2 of 3");
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

    openFullFamily();
    expect(screen.getByTestId("agentic-tool-message")).toBeInTheDocument();
    expect(
      within(screen.getByTestId("agentic-tool-message")).getByText(
        "Rebase please",
      ),
    ).toBeInTheDocument();
    expect(
      within(screen.getByLabelText("Agent message receipt")).getByText(
        "peer-1",
      ),
    ).toBeInTheDocument();
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

    openFullFamily();
    expect(screen.getByTestId("agentic-tool-agents")).toBeInTheDocument();
    expect(screen.getByText("Fix the pond")).toBeInTheDocument();
  });

  test("shows pending state when no result arrived", () => {
    renderTools([], [toolCall("agent_wait", { agent_id: "abc" })]);

    openFullFamily();
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
    openFullFamily();
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
    openFullFamily();
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

    openFullFamily();
    expect(screen.getByTestId("agentic-tool-error")).toBeInTheDocument();
  });

  test("routes subagent previews through the dedicated context card", () => {
    renderTools([launchPreviewReceipt()], [launchPreviewCall]);
    fireEvent.click(screen.getByRole("button", { expanded: false }));
    fireEvent.click(screen.getByRole("button", { name: "More details" }));
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

    openFullFamily();
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

describe("AgenticTool collaboration receipts", () => {
  test.each([0, 1])(
    "metrics shows %i typed snapshot entries without claiming an agent roster",
    (count) => {
      const entries = Array.from({ length: count }, () => ({
        kind: "series",
        series: {
          name: "evidence_verdicts",
          labels: [{ name: "verdict", value: "blocked" }],
          sample: { kind: "counter", value: 4 },
        },
      }));
      const content = JSON.stringify({
        snapshot_id: "snapshot-1",
        seq: 0,
        project: "project-1",
        entries,
        total: entries.length,
        series_total: entries.length,
        explanations_total: 0,
        returned: entries.length,
        truncated: false,
        generated_at_ms: 1,
      });
      renderTools([toolResult("metrics", content)], [toolCall("metrics")]);
      expect(screen.getByText("Project metrics checked")).toBeInTheDocument();
      expect(screen.getByRole("status")).toHaveTextContent("Snapshot received");
      fireEvent.click(
        screen.getByRole("button", { name: "Project metrics checked" }),
      );
      expect(
        screen.getByText("Entries shown").nextElementSibling,
      ).toHaveTextContent(String(entries.length));
      expect(
        screen.getByText("Metric series").nextElementSibling,
      ).toHaveTextContent(String(entries.length));
      expect(
        screen.getByText("Decision explanations").nextElementSibling,
      ).toHaveTextContent("0");
      expect(screen.queryByText("Roster snapshot")).toBeNull();
      expect(
        screen.queryByText("No agents were listed in this snapshot."),
      ).toBeNull();
    },
  );

  test.each([
    [
      "team_role_grant",
      "Granted Mail authority through `ship-it` role `implementer`.",
    ],
    [
      "code_mode",
      "code_mode authorized 0 host calls\nIntermediate bytes: 0 / 1000",
    ],
    ["code_mode", "Arbitrary unknown short response"],
  ])(
    "%s keeps untyped %s unconfirmed without control acceptance",
    (name, content) => {
      renderTools([toolResult(name, content)], [toolCall(name)]);
      expect(screen.getByRole("status")).toHaveTextContent(
        "Outcome unconfirmed",
      );
      expect(screen.queryByText("Control request accepted")).toBeNull();
    },
  );

  test.each(LOCAL_COLLABORATION_TOOL_NAMES)(
    "%s routes only through the collaboration view, including aliases",
    (name) => {
      const pascal = name
        .split("_")
        .map((part) => part[0].toUpperCase() + part.slice(1))
        .join("");
      for (const alias of [
        name,
        `t_${name}`,
        pascal,
        `T_${pascal}`,
        name.replace(/_/gu, "-"),
      ]) {
        const { container, unmount } = renderTools(
          [toolResult(alias, "Authorized and verified!")],
          [toolCall(alias)],
        );
        expect(
          screen.getByLabelText("Collaboration receipt"),
        ).toBeInTheDocument();
        expect(screen.getByRole("status")).toHaveTextContent(
          "Outcome unconfirmed",
        );
        const outer = within(
          screen.getByLabelText("Collaboration receipt"),
        ).getAllByRole("button")[0];
        expect(outer).toHaveAttribute("aria-expanded", "false");
        fireEvent.click(outer);
        expect(
          screen.getByRole("button", { name: "More details" }),
        ).toHaveAttribute("aria-expanded", "false");
        fireEvent.click(screen.getByRole("button", { name: "More details" }));
        expect(screen.queryByText("Control request accepted")).toBeNull();
        expect(screen.queryByText("Roster snapshot")).toBeNull();
        expect(screen.queryByLabelText("Task memory receipt")).toBeNull();
        expect(container.querySelectorAll("dl dd").length).toBeLessThanOrEqual(
          3,
        );
        unmount();
      }
    },
  );

  test("team seat uses its structured receipt instead of a task board outcome", () => {
    const args = { team: "ship-it", role: "implementer", action: "claim" };
    const receipt = {
      ...args,
      member_chat_id: "chat-1",
      result: "claimed",
      outcome: null,
      members: [],
    };
    renderTools(
      [
        {
          ...toolResult(
            "team_seat",
            `Team \`ship-it\` role \`implementer\` claimed for this chat. 0 live team member(s).\n\n${JSON.stringify(
              receipt,
            )}`,
          ),
          extra: { team_seat: receipt },
        },
      ],
      [toolCall("team_seat", args)],
    );
    expect(
      screen.getByText("Team seat claim recorded: ship-it"),
    ).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent("Seat receipt");
    expect(screen.queryByLabelText("Task board receipt")).toBeNull();
  });

  test("role grants disclose historical receipt rather than current authority", () => {
    const args = {
      team: "ship-it",
      role: "implementer",
      authority: "mail",
      member_agent_id: "agent-1",
      recipient_chat_id: "chat-2",
    };
    const grant = { ...args, member_chat_id: null, recipient_agent_id: null };
    renderTools(
      [
        {
          ...toolResult(
            "team_role_grant",
            "Granted Mail authority through `ship-it` role `implementer`.",
          ),
          extra: { team_role_grant: grant },
        },
      ],
      [toolCall("team_role_grant", args)],
    );
    expect(screen.getByText("Role grant receipt recorded")).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent(
      "Historical grant receipt",
    );
    fireEvent.click(
      screen.getByRole("button", { name: "Role grant receipt recorded" }),
    );
    expect(
      screen.getByText("Historical receipt; not current permission"),
    ).toBeInTheDocument();
    expect(screen.queryByText("Control request accepted")).toBeNull();
  });

  test.each([
    ["claim_record", "Claim recorded.\n\nClaim `c-1` [active]"],
    ["claim_list", "Claim `c-1` [active]"],
  ])(
    "%s is an unconfirmed assertion, not a task memory note",
    (name, content) => {
      renderTools(
        [toolResult(name, content)],
        [
          toolCall(name, {
            subject: "work:T-1",
            predicate: "is",
            assertion: "done",
          }),
        ],
      );
      expect(screen.getByRole("status")).toHaveTextContent(
        "Outcome unconfirmed",
      );
      expect(screen.queryByLabelText("Task memory receipt")).toBeNull();
      expect(screen.queryByText("Memory notes")).toBeNull();
    },
  );

  test.each([
    ["metrics", "transport failed", "Request failed"],
    [
      "code_mode",
      "code mode is unavailable: the `code_mode_enabled` setting is off",
      "Unavailable",
    ],
    ["team_role_grant", "Everything authorized!", "Request failed"],
  ])("%s keeps transport failure explicit", (name, content, label) => {
    const { container } = renderTools(
      [toolResult(name, content, true)],
      [toolCall(name)],
    );
    expect(screen.getByRole("alert")).toHaveTextContent(label);
    expect(container.querySelector("section")).toHaveAttribute(
      "data-status",
      "error",
    );
    expect(screen.queryByText("Control request accepted")).toBeNull();
    expect(screen.queryByText("Roster snapshot")).toBeNull();
  });

  test("structured denial is explicit even when transport succeeded", () => {
    renderTools(
      [
        toolResult(
          "board_claim",
          JSON.stringify({
            code: "scope_denied",
            message: "Request refused",
            retryable: false,
            request_id: "request-1",
          }),
        ),
      ],
      [toolCall("board_claim", { card_id: "T-1" })],
    );
    expect(screen.getByRole("alert")).toHaveTextContent("Request denied");
    fireEvent.click(
      screen.getByRole("button", { name: "Claim requested: T-1" }),
    );
    expect(screen.getByText("Request refused")).toBeInTheDocument();
    expect(screen.queryByText("Card claim recorded: T-1")).toBeNull();
  });

  test("missing collaboration responses stay awaiting without claiming success", () => {
    renderTools([], [toolCall("metrics")]);
    expect(screen.getByRole("status")).toHaveTextContent("Awaiting result");
    expect(screen.queryByText("Project metrics checked")).toBeNull();
    expect(screen.queryByText("Roster snapshot")).toBeNull();
  });

  test("oversized discovery stays unconfirmed while every raw page remains recoverable", () => {
    const content = "projection_unavailable: false\ncards: []\n#".padEnd(
      256_001,
      "x",
    );
    const message = toolResult("board_discover", content);
    renderTools([message], [toolCall("board_discover")]);
    expect(screen.getByText("Board discovery requested")).toBeInTheDocument();
    expect(screen.getByText("Outcome unconfirmed")).toHaveAttribute(
      "role",
      "status",
    );
    expect(screen.queryByText("Cards shown")).toBeNull();
    fireEvent.click(
      screen.getByRole("button", { name: "Board discovery requested" }),
    );
    fireEvent.click(screen.getByRole("button", { name: "More details" }));
    const response = screen.getByRole("region", { name: "Full response" });
    const pages = within(response);
    let recovered =
      pages.getByLabelText("Full response content").textContent ?? "";
    const next = pages.getByRole("button", { name: "Next section" });
    for (
      let index = 1;
      index < Math.ceil(content.length / PAGE_CHARS);
      index++
    ) {
      fireEvent.click(next);
      recovered +=
        pages.getByLabelText("Full response content").textContent ?? "";
    }
    expect(recovered).toBe(content);
    expect(message.content).toBe(content);
    expect(next).toBeDisabled();
  });

  test("disclosure preserves complete original arguments, response and metadata without legacy views", () => {
    const call = toolCall("code_mode", { script: "print('hello')" });
    call.function.arguments = '{ "script" : "print(1)" }';
    const content =
      "first page\n" +
      "x".repeat(PAGE_CHARS) +
      "\nlast page <script>not executed</script>";
    const extra = { diagnostic: "metadata kept", nested: { count: 2 } };
    const message = { ...toolResult("code_mode", content), extra };
    const { container } = renderTools([message], [call]);
    fireEvent.click(
      screen.getByRole("button", { name: "Orchestration requested" }),
    );
    const disclosure = screen.getByRole("button", { name: "More details" });
    expect(disclosure).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByText("Control request accepted")).toBeNull();
    fireEvent.click(disclosure);
    expect(screen.getByLabelText("Raw arguments content").textContent).toBe(
      call.function.arguments,
    );
    const response = screen.getByRole("region", { name: "Full response" });
    const firstPage = within(response).getByLabelText(
      "Full response content",
    ).textContent;
    fireEvent.click(
      within(response).getByRole("button", { name: "Next section" }),
    );
    const secondPage = within(response).getByLabelText(
      "Full response content",
    ).textContent;
    expect(`${firstPage}${secondPage}`).toBe(content);
    expect(
      JSON.parse(
        screen.getByLabelText("Response metadata content").textContent ?? "",
      ),
    ).toEqual(extra);
    expect(container.querySelector("script")).toBeNull();
    expect(message.content).toBe(content);
    expect(message.extra).toBe(extra);
    expect(call.function.arguments).toBe('{ "script" : "print(1)" }');
  });
});

describe("AgenticTool collaboration compact shell", () => {
  test("defaults closed with an outline icon, then opens facts and full details independently by keyboard", async () => {
    const call = toolCall("board_claim", { card_id: "T-1", task_id: "task-1" });
    const { user, container } = renderTools(
      [toolResult("board_claim", "Unconfirmed response")],
      [call],
    );
    const card = screen.getByLabelText("Collaboration receipt");
    const outer = screen.getByRole("button", { name: "Claim requested: T-1" });
    const details = screen.getByRole("button", { name: "More details" });
    const body = document.getElementById(
      outer.getAttribute("aria-controls") ?? "",
    );
    const fullBody = document.getElementById(
      details.getAttribute("aria-controls") ?? "",
    );

    expect(card).toHaveAttribute("data-density", "compact");
    expect(card).toHaveAttribute("data-status", "idle");
    expect(outer).toHaveAttribute("aria-expanded", "false");
    expect(body).toHaveAttribute("inert");
    expect(details).toHaveAttribute("aria-expanded", "false");
    expect(details).toHaveAttribute("tabindex", "-1");
    expect(fullBody).toHaveAttribute("inert");
    expect(outer.querySelectorAll("svg")).toHaveLength(2);
    expect(outer.querySelector("svg")).toHaveAttribute("fill", "none");
    expect(outer.querySelector("svg")).toHaveAttribute("stroke-width", "1.5");
    expect(
      container.querySelectorAll('section[data-density="compact"]'),
    ).toHaveLength(1);
    expect(screen.getByRole("status")).toHaveTextContent("Outcome unconfirmed");

    await user.tab();
    expect(outer).toHaveFocus();
    await user.keyboard("{Enter}");

    expect(outer).toHaveAttribute("aria-expanded", "true");
    expect(body).not.toHaveAttribute("inert");
    expect(card.querySelectorAll("dl dd").length).toBeLessThanOrEqual(3);
    expect(details).toHaveAttribute("aria-expanded", "false");
    expect(fullBody).toHaveAttribute("inert");
    await user.tab();
    expect(details).toHaveFocus();
    await user.keyboard(" ");

    expect(details).toHaveAttribute("aria-expanded", "true");
    expect(fullBody).not.toHaveAttribute("inert");
    expect(screen.getByLabelText("Raw arguments content").textContent).toBe(
      call.function.arguments,
    );
    expect(screen.getByLabelText("Full response content").textContent).toBe(
      "Unconfirmed response",
    );
    outer.focus();
    await user.keyboard(" ");
    expect(outer).toHaveAttribute("aria-expanded", "false");
    expect(details).toHaveAttribute("aria-expanded", "true");
    expect(details).toHaveAttribute("tabindex", "-1");
    await user.keyboard("{Enter}");
    expect(details).toHaveAttribute("aria-expanded", "true");
  });

  test.each([true, false])(
    "preserves explicit tc:id choice %s across rerenders, siblings and remounts",
    (storedOpen) => {
      const call = toolCall("metrics");
      const sibling = toolCall("code_mode");
      const key = `tc:${call.id}`;
      const choices = new Map([[key, storedOpen]]);
      const preferences = {
        get: (storeKey: string) => choices.get(storeKey),
        set: (storeKey: string, open: boolean) => {
          choices.set(storeKey, open);
        },
      };
      const wrap = (content: ReactElement) => (
        <CollapsibleStoreProvider value={preferences}>
          {content}
        </CollapsibleStoreProvider>
      );
      const calls = [call, sibling];
      const { rerender, unmount } = renderTools([], calls, wrap);
      const first = screen.getByRole("button", {
        name: "Project metrics requested",
      });
      const second = screen.getByRole("button", {
        name: "Orchestration requested",
      });
      const bodyId = first.getAttribute("aria-controls");
      expect(first).toHaveAttribute("aria-expanded", String(storedOpen));
      expect(second).toHaveAttribute("aria-expanded", "false");
      fireEvent.click(second);
      expect(first).toHaveAttribute("aria-expanded", String(storedOpen));
      expect(choices.get(`tc:${sibling.id}`)).toBe(true);
      rerender(
        wrap(
          <ToolContent
            toolCalls={[
              { ...call, function: { ...call.function, name: "T_Metrics" } },
              sibling,
            ]}
            threadId={CHAT_ID}
          />,
        ),
      );
      expect(first).toHaveAttribute("aria-expanded", String(storedOpen));
      expect(first).toHaveAttribute("aria-controls", bodyId);
      fireEvent.click(first);
      expect(choices.get(key)).toBe(!storedOpen);
      unmount();
      renderTools([], calls, wrap);
      expect(
        screen.getByRole("button", { name: "Project metrics requested" }),
      ).toHaveAttribute("aria-expanded", String(!storedOpen));
      expect(
        screen.getByRole("button", { name: "Orchestration requested" }),
      ).toHaveAttribute("aria-expanded", "true");
    },
  );

  test("streaming, failures and later results never open a closed collaboration row", () => {
    const call = toolCall("metrics");
    const { store } = renderTools([], [call]);
    const outer = screen.getByRole("button", {
      name: "Project metrics requested",
    });
    const card = screen.getByLabelText("Collaboration receipt");
    act(() => {
      store.dispatch(
        applyChatEvent({
          chat_id: CHAT_ID,
          seq: "1",
          type: "runtime_updated",
          state: "generating",
        }),
      );
    });
    expect(screen.getByRole("status")).toHaveTextContent("Running");
    expect(card).toHaveAttribute("data-status", "running");
    expect(outer).toHaveAttribute("aria-expanded", "false");
    act(() => {
      store.dispatch(
        applyChatEvent({
          chat_id: CHAT_ID,
          seq: "2",
          type: "message_added",
          index: 0,
          message: {
            ...toolResult("metrics", "transport failed", true),
            message_id: "metrics-result",
          },
        }),
      );
    });
    expect(screen.getByRole("alert")).toHaveTextContent("Request failed");
    expect(card).toHaveAttribute("data-status", "error");
    expect(outer).toHaveAttribute("aria-expanded", "false");
    act(() => {
      store.dispatch(
        applyChatEvent({
          chat_id: CHAT_ID,
          seq: "3",
          type: "message_updated",
          message_id: "metrics-result",
          message: {
            ...toolResult("metrics", "Unknown result"),
            message_id: "metrics-result",
          },
        }),
      );
    });
    expect(screen.getByRole("status")).toHaveTextContent("Outcome unconfirmed");
    expect(card).toHaveAttribute("data-status", "idle");
    expect(outer).toHaveAttribute("aria-expanded", "false");
  });

  test("prepares pointer, mouse and keyboard anchors before preserving the toggle mutation", () => {
    const order: string[] = [];
    const { container } = renderTools([], [toolCall("metrics")], (content) => (
      <ChatScrollAnchorContext.Provider
        value={{
          prepareScrollAnchor: () => {
            order.push("prepare");
          },
          preserveScrollAnchor: (mutate) => {
            order.push("preserve");
            mutate();
          },
        }}
      >
        {content}
      </ChatScrollAnchorContext.Provider>
    ));
    const outer = screen.getByRole("button", {
      name: "Project metrics requested",
    });
    for (const prepare of [
      fireEvent.pointerDown,
      fireEvent.mouseDown,
      fireEvent.keyDown,
    ]) {
      order.length = 0;
      prepare(outer);
      fireEvent.click(outer);
      expect(order).toEqual(["prepare", "preserve"]);
    }
    expect(
      container.querySelectorAll('section[data-density="compact"]'),
    ).toHaveLength(1);
  });

  test("retains the real tool-call tooltip and untouched raw arguments", () => {
    vi.useFakeTimers();
    try {
      const call = toolCall("T_CodeMode");
      call.function.arguments = '{ "script" : "print(1)" }';
      renderTools([], [call]);
      const card = screen.getByLabelText("Collaboration receipt");
      expect(card.parentElement).not.toBeNull();
      if (card.parentElement) fireEvent.mouseEnter(card.parentElement);
      act(() => {
        vi.advanceTimersByTime(10_000);
      });
      expect(screen.getByText("script")).toBeInTheDocument();
      expect(screen.getByText("print(1)")).toBeInTheDocument();
      expect(call.function.arguments).toBe('{ "script" : "print(1)" }');
      expect(
        screen.getByRole("button", { name: "Orchestration requested" }),
      ).toHaveAttribute("aria-expanded", "false");
    } finally {
      vi.useRealTimers();
    }
  });

  test("keeps secondary disclosure and page state mounted across outer toggles", () => {
    const content = "x".repeat(PAGE_CHARS) + "last page";
    renderTools([toolResult("code_mode", content)], [toolCall("code_mode")]);
    const outer = screen.getByRole("button", {
      name: "Orchestration requested",
    });
    const details = screen.getByRole("button", { name: "More details" });
    fireEvent.click(outer);
    fireEvent.click(details);
    const response = within(
      screen.getByRole("region", { name: "Full response" }),
    );
    fireEvent.click(response.getByRole("button", { name: "Next section" }));
    fireEvent.click(outer);
    expect(outer).toHaveAttribute("aria-expanded", "false");
    fireEvent.click(outer);
    expect(details).toHaveAttribute("aria-expanded", "true");
    expect(response.getByRole("status")).toHaveTextContent("Section 2 of 2");
    expect(response.getByLabelText("Full response content")).toHaveTextContent(
      "last page",
    );
  });

  test("opts non-collaboration into the same closed compact default", () => {
    renderTools(
      [toolResult("agent_cancel", "Cancel requested")],
      [toolCall("agent_cancel")],
    );
    const outer = screen.getAllByRole("button")[0];
    expect(outer).toHaveAttribute("aria-expanded", "false");
    expect(outer.closest("section")).toHaveAttribute("data-density", "compact");
  });
});

describe("composed compact family inventory", () => {
  test.each(INVENTORY)(
    "%s stays compact, closed and evidence-limited for pending, malformed and failed history",
    (name) => {
      for (const phase of ["pending", "malformed", "failed"] as const) {
        const call = toolCall(name);
        call.function.arguments =
          phase === "malformed"
            ? "{broken"
            : JSON.stringify({
                title: "Requested work",
                task: "Requested work",
                text: "Keep scope",
                agent_id: "raw-agent-id",
                card_id: "T-1",
              });
        const { container, unmount } = renderTools(
          phase === "pending"
            ? []
            : [
                toolResult(
                  name,
                  "RAW_TECHNICAL_SENTINEL completed verified pushed delivered",
                  phase === "failed",
                ),
              ],
          [call],
        );
        const row = container.querySelector('section[data-density="compact"]');
        expect(row, name).not.toBeNull();
        expect(row).toHaveAttribute("data-open", "false");
        expect(row?.querySelector("button")).not.toHaveTextContent(
          /RAW_TECHNICAL_SENTINEL|subagent:|agent message →/iu,
        );
        expect(screen.queryByTestId("generic-tool")).toBeNull();
        const details = screen.getByRole("button", { name: "More details" });
        expect(details).toHaveAttribute("aria-expanded", "false");
        expect(details.closest("[inert]")).not.toBeNull();
        if (phase === "failed")
          expect(row).toHaveAttribute("data-status", "error");
        else expect(row).toHaveAttribute("data-status", "idle");
        const outer = row?.querySelector("button");
        if (!outer) throw new Error("Missing compact toggle");
        fireEvent.click(outer);
        expect(
          Array.from(row?.querySelectorAll("dl dd") ?? []).filter(
            (fact) => !fact.closest("[inert]"),
          ).length,
        ).toBeLessThanOrEqual(3);
        expect(details).toHaveAttribute("aria-expanded", "false");
        fireEvent.click(details);
        expect(screen.getByLabelText("Raw arguments content").textContent).toBe(
          call.function.arguments,
        );
        if (phase !== "pending")
          expect(
            screen.getByLabelText("Full response content"),
          ).toHaveTextContent("RAW_TECHNICAL_SENTINEL");
        unmount();
      }
    },
  );

  test.each(["agent_message", "set_plan", "agent_wait", "board_update"])(
    "%s preserves independent stored choices, navigation focus and stream updates",
    async (name) => {
      const call = toolCall(name, {
        text: "Requested content",
        content: "Requested content",
        agent_id: "abc",
        card_id: "T-1",
      });
      const sibling = toolCall("code_mode");
      const values = new Map([[`tc:${call.id}`, true]]);
      const storage = {
        get: (key: string) => values.get(key),
        set: (key: string, value: boolean) => {
          values.set(key, value);
        },
      };
      const wrap = (content: ReactElement) => (
        <CollapsibleStoreProvider value={storage}>
          {content}
        </CollapsibleStoreProvider>
      );
      const { container, user, store, unmount } = renderTools(
        [],
        [call, sibling],
        wrap,
      );
      const rows = container.querySelectorAll(
        'section[data-density="compact"]',
      );
      const outer = rows[0].querySelector("button");
      const other = rows[1].querySelector("button");
      if (!outer || !other) throw new Error("Missing sibling toggles");
      expect(outer).toHaveAttribute("aria-expanded", "true");
      expect(other).toHaveAttribute("aria-expanded", "false");
      fireEvent.click(outer);
      outer.focus();
      await user.tab();
      expect(other).toHaveFocus();
      act(() => {
        store.dispatch(
          applyChatEvent({
            type: "runtime_updated",
            chat_id: CHAT_ID,
            seq: "1",
            state: "generating",
          }),
        );
        store.dispatch(
          applyChatEvent({
            type: "message_added",
            chat_id: CHAT_ID,
            seq: "2",
            index: 0,
            message: {
              ...toolResult(name, "failed transport", true),
              message_id: "compact-result",
            },
          }),
        );
      });
      expect(outer).toHaveAttribute("aria-expanded", "false");
      expect(rows[0]).toHaveAttribute("data-status", "error");
      expect(other).toHaveAttribute("aria-expanded", "false");
      unmount();
      const remounted = renderTools([], [call, sibling], wrap);
      expect(
        remounted.container.querySelector('section[data-density="compact"]'),
      ).toHaveAttribute("data-open", "false");
    },
  );

  test("typed mail metadata drives both compact receipt and retained full receipt", () => {
    const message = {
      ...toolResult("agent_message", "Misleading prose: delivered"),
      extra: {
        delivery_receipt: {
          kind: "agent_message",
          delivery_id: "delivery-1",
          outcome: "queued",
          ttl_secs: 15,
        },
      },
    };
    renderTools(
      [message],
      [toolCall("agent_message", { text: "Wait for the check", to: "abc" })],
    );
    expect(screen.getByRole("status")).toHaveTextContent("Queued");
    openFullFamily();
    expect(screen.getByLabelText("Agent message receipt")).toHaveTextContent(
      "Message queued (expires after 15s unless landed)",
    );
    expect(
      screen.getByLabelText("Response metadata content"),
    ).toHaveTextContent('"outcome": "queued"');
  });

  test("hostile huge arguments stay inert and bounded with complete deep recovery", () => {
    const task = "<script>attack()</script>" + "🦊".repeat(5000);
    const call = toolCall("set_plan", { content: task });
    const { container } = renderTools(
      [toolResult("set_plan", '{"version":1,"supersedes":null}')],
      [call],
    );
    const outer = container.querySelector("button");
    expect(Array.from(outer?.textContent ?? "").length).toBeLessThanOrEqual(
      180,
    );
    expect(container.querySelector("script")).toBeNull();
    openFullFamily();
    const raw = within(screen.getByRole("region", { name: "Raw arguments" }));
    let recovered =
      raw.getByLabelText("Raw arguments content").textContent ?? "";
    const next = raw.getByRole("button", { name: "Next section" });
    for (
      let index = 1;
      index < Math.ceil(call.function.arguments.length / PAGE_CHARS);
      index++
    ) {
      fireEvent.click(next);
      recovered +=
        raw.getByLabelText("Raw arguments content").textContent ?? "";
    }
    expect(recovered).toBe(call.function.arguments);
  });
});
