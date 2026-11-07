import { describe, expect, it } from "vitest";
import { act as rtlAct } from "@testing-library/react";
import {
  createDefaultChatState,
  fireEvent,
  render,
  screen,
  within,
} from "../../utils/test-utils";
import type { ToolCall, ToolMessage } from "../../services/refact/types";
import { applyChatEvent } from "../../features/Chat/Thread";
import { CollapsibleStoreProvider } from "./useStoredOpen";
import { parseAgentDiffOutput } from "./AgentDiffModel";
import { AgentDiffContent, AgentDiffView } from "./AgentDiffView";

const act = rtlAct as unknown as (callback: () => void) => void;

const TOOL_CALL: ToolCall = {
  id: "diff-call",
  index: 0,
  function: { name: "agent_diff", arguments: '{"card_id":"T-29"}' },
};

function diffMessage(content: string, tool_failed = false): ToolMessage {
  return {
    role: "tool",
    message_id: "diff-result",
    tool_call_id: "diff-call",
    content,
    tool_failed,
  };
}

function renderDiff(
  content: string | null,
  toolFailed = false,
  waiting = false,
) {
  const chat = createDefaultChatState();
  const threadId = chat.current_thread_id;
  chat.threads[threadId].thread.messages =
    content === null
      ? [{ role: "assistant", content: "", tool_calls: [TOOL_CALL] }]
      : [diffMessage(content, toolFailed)];
  chat.threads[threadId].waiting_for_response = waiting;
  return {
    ...render(<AgentDiffView toolCall={TOOL_CALL} />, {
      preloadedState: { chat },
    }),
    threadId,
  };
}

const FENCE = "```";
const DIFF_OUTPUT = [
  "# Agent Diff for T-29",
  "",
  "**Card:** check-agents redesign",
  "**Branch:** refact/task/T-29-agent",
  "**Base:** commit abc123",
  "",
  `${FENCE}diff`,
  "## Committed changes since base",
  "diff --git a/src/tools/tool_task_check_agents.rs b/src/tools/tool_task_check_agents.rs",
  "index 1111111..2222222 100644",
  "--- a/src/tools/tool_task_check_agents.rs",
  "+++ b/src/tools/tool_task_check_agents.rs",
  "@@ -1,2 +1,2 @@",
  "-old line",
  "+new line",
  "",
  "diff --git a/src/components/ChatContent/AgentPulseView.tsx b/src/components/ChatContent/AgentPulseView.tsx",
  "new file mode 100644",
  "--- /dev/null",
  "+++ b/src/components/ChatContent/AgentPulseView.tsx",
  "@@ -0,0 +1,2 @@",
  "+export const AgentPulseView = true;",
  "+export default AgentPulseView;",
  "... (4 more lines, use mode='name-only' to see all files)",
  FENCE,
].join("\n");

const RENAME_ONLY_DIFF_OUTPUT = [
  "# Agent Diff for T-30",
  "",
  "**Card:** rename-only card",
  "**Branch:** refact/task/T-30-agent",
  "**Base:** commit def456",
  "",
  `${FENCE}diff`,
  "diff --git a/src/old-name.ts b/src/new-name.ts",
  "similarity index 100%",
  "rename from src/old-name.ts",
  "rename to src/new-name.ts",
  "",
  "diff --git a/src/changed.ts b/src/changed.ts",
  "index 1111111..2222222 100644",
  "--- a/src/changed.ts",
  "+++ b/src/changed.ts",
  "@@ -1 +1 @@",
  "-old value",
  "+new value",
  FENCE,
].join("\n");

describe("AgentDiffView parsing", () => {
  it("parses unified agent diff markdown", () => {
    const report = parseAgentDiffOutput(DIFF_OUTPUT);

    expect(report).toMatchObject({
      cardId: "T-29",
      cardTitle: "check-agents redesign",
      branch: "refact/task/T-29-agent",
      base: "commit abc123",
      mode: "unified",
      truncated: "... (4 more lines, use mode='name-only' to see all files)",
    });
    expect(report?.files).toEqual([
      "src/tools/tool_task_check_agents.rs",
      "src/components/ChatContent/AgentPulseView.tsx",
    ]);
    expect(report?.stats).toMatchObject({ files: 2, added: 3, removed: 1 });
    expect(report?.diffChunks).toHaveLength(2);
  });

  it("parses unified diffs containing markdown fences", () => {
    const output = [
      "# Agent Diff for T-30",
      "",
      "**Card:** docs update",
      "**Branch:** refact/task/T-30-agent",
      "**Base:** commit def456",
      "",
      `${FENCE}diff`,
      "## Committed changes since base",
      "diff --git a/README.md b/README.md",
      "index 1111111..2222222 100644",
      "--- a/README.md",
      "+++ b/README.md",
      "@@ -1,1 +1,5 @@",
      " # Usage",
      "+Run this command:",
      "+```bash",
      "+npm test",
      "+```",
      FENCE,
    ].join("\n");

    const report = parseAgentDiffOutput(output);

    expect(report?.body).toContain("+```bash");
    expect(report?.body).toContain("+```");
    expect(report?.files).toEqual(["README.md"]);
    expect(report?.stats).toMatchObject({ files: 1, added: 4, removed: 0 });
    expect(report?.diffChunks[0]?.lines_add).toContain("```bash");
  });
});

describe("AgentDiffContent", () => {
  it("renders diff output with file tree and truncation banner", () => {
    const report = parseAgentDiffOutput(DIFF_OUTPUT);
    if (!report) throw new Error("expected diff report");

    render(<AgentDiffContent report={report} />);

    expect(
      screen.getByText(
        "... (4 more lines, use mode='name-only' to see all files)",
      ),
    ).toBeInTheDocument();
  });

  it("filters rendered unified diff by selected file", () => {
    renderDiff(DIFF_OUTPUT);
    const outer = screen.getByTestId("transcript-toggle");
    expect(outer).toHaveAttribute("aria-expanded", "true");

    fireEvent.click(
      screen.getAllByText("src/components/ChatContent/AgentPulseView.tsx")[0],
    );

    expect(
      screen.getByText("export const AgentPulseView = true;"),
    ).toBeInTheDocument();
    expect(screen.queryByText("new line")).not.toBeInTheDocument();
    expect(outer).toHaveAttribute("aria-expanded", "true");
  });

  it("renders a selected file with no hunks without throwing and shows empty diff message", () => {
    const report = parseAgentDiffOutput(RENAME_ONLY_DIFF_OUTPUT);
    if (!report) throw new Error("expected diff report");

    render(<AgentDiffContent report={report} />);

    fireEvent.click(screen.getByText("src/new-name.ts"));

    expect(
      screen.getByText("No diff hunks for this file."),
    ).toBeInTheDocument();
    expect(screen.queryByText("old value")).not.toBeInTheDocument();
  });
});

function itemOf(toggle: HTMLElement): HTMLElement {
  const section = toggle.closest("section");
  if (!section) throw new Error("expected transcript section");
  return section;
}

describe("AgentDiffView transcript route", () => {
  it("opens by default with the file count, card title and partial coverage", async () => {
    const { user } = renderDiff(DIFF_OUTPUT);
    const outer = screen.getByTestId("transcript-toggle");
    expect(outer).toHaveAttribute("aria-expanded", "true");
    expect(itemOf(outer)).toHaveAttribute("data-status", "done");
    expect(within(outer).getByTestId("transcript-verb")).toHaveTextContent(
      "Diff",
    );
    expect(within(outer).getByTestId("transcript-target")).toHaveTextContent(
      "check-agents redesign",
    );
    expect(within(outer).getByTestId("transcript-meta")).toHaveTextContent(
      "2 files · partial",
    );
    expect(outer).not.toHaveTextContent("T-29");
    expect(screen.queryByTestId("transcript-status-pill")).toBeNull();
    const files = screen.getByRole("button", { name: "All files" });
    expect(files.closest("[inert]")).toBeNull();
    expect(screen.getByText("refact/task/T-29-agent")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Raw" }));
    const blocks = Array.from(
      screen.getByTestId("transcript-raw").querySelectorAll("pre"),
    ).map((pre) => pre.textContent);
    expect(blocks).toEqual([
      JSON.stringify({ card_id: "T-29" }, null, 2),
      DIFF_OUTPUT,
    ]);
    await user.click(outer);
    expect(outer).toHaveAttribute("aria-expanded", "false");
  });

  it("omits the pill for a complete diff", () => {
    renderDiff(RENAME_ONLY_DIFF_OUTPUT);
    const outer = screen.getByTestId("transcript-toggle");
    expect(within(outer).getByTestId("transcript-meta")).toHaveTextContent(
      "2 files",
    );
    expect(
      screen.queryByTestId("transcript-status-pill"),
    ).not.toBeInTheDocument();
  });

  it.each([
    [null, false, false, "unknown", null, "No result"],
    [null, false, true, "running", null, "Running"],
    ["not a diff", false, false, "done", null, null],
    ['agent_diff(card_id="T-29")', true, false, "failed", null, "Failed"],
    [
      `# Agent Diff for T-29\n${FENCE}diff\n${FENCE}`,
      false,
      false,
      "done",
      "no changes",
      null,
    ],
    [
      `# Agent Diff for T-29\n${FENCE}\n(no changes detected)\n${FENCE}`,
      false,
      false,
      "done",
      "no changes",
      null,
    ],
  ])(
    "does not invent changes or request success: %s",
    (content, failed, waiting, status, meta, pillText) => {
      renderDiff(content, failed, waiting);
      const outer = screen.getByTestId("transcript-toggle");
      expect(outer).toHaveAttribute("aria-expanded", "false");
      expect(itemOf(outer)).toHaveAttribute("data-status", status);
      const metaNode = within(outer).queryByTestId("transcript-meta");
      if (meta === null) expect(metaNode).not.toBeInTheDocument();
      else expect(metaNode).toHaveTextContent(meta);
      const pill = screen.queryByTestId("transcript-status-pill");
      if (pillText === null) expect(pill).not.toBeInTheDocument();
      else expect(pill).toHaveTextContent(pillText);
    },
  );

  it("shows a failed request in the Error section", async () => {
    const { user } = renderDiff("diff failed sentinel", true);
    await user.click(screen.getByTestId("transcript-toggle"));
    expect(screen.getByRole("alert")).toHaveTextContent("diff failed sentinel");
    expect(screen.queryByText("All files")).not.toBeInTheDocument();
  });

  it("honors stored open and closed choices across state changes and remounts", () => {
    const preferences = new Map([
      ["agent_diff:diff-call", true],
      ["agent_diff:second-diff-call", false],
    ]);
    const collapseStore = {
      get: (key: string) => preferences.get(key),
      set: (key: string, value: boolean) => {
        preferences.set(key, value);
      },
    };
    const chat = createDefaultChatState();
    const threadId = chat.current_thread_id;
    chat.threads[threadId].thread.messages = [diffMessage(DIFF_OUTPUT)];
    const view = () => (
      <CollapsibleStoreProvider value={collapseStore}>
        <AgentDiffView toolCall={TOOL_CALL} />
        <AgentDiffView toolCall={{ ...TOOL_CALL, id: "second-diff-call" }} />
      </CollapsibleStoreProvider>
    );
    const { store, unmount } = render(view(), { preloadedState: { chat } });
    const [outer, sibling] = screen.getAllByTestId("transcript-toggle");
    expect(outer).toHaveAttribute("aria-expanded", "true");
    fireEvent.click(
      screen.getByRole("button", {
        name: "src/components/ChatContent/AgentPulseView.tsx",
      }),
    );
    expect(sibling).toHaveAttribute("aria-expanded", "false");
    act(() => {
      store.dispatch(
        applyChatEvent({
          chat_id: threadId,
          seq: "1",
          type: "message_added",
          index: 0,
          message: diffMessage(DIFF_OUTPUT.replace("new line", "changed line")),
        }),
      );
    });
    expect(outer).toHaveAttribute("aria-expanded", "true");
    expect(screen.queryByText("changed line")).not.toBeInTheDocument();
    fireEvent.click(outer);
    act(() => {
      store.dispatch(
        applyChatEvent({
          chat_id: threadId,
          seq: "2",
          type: "message_added",
          index: 0,
          message: diffMessage("request failed", true),
        }),
      );
    });
    expect(outer).toHaveAttribute("aria-expanded", "false");
    expect(sibling).toHaveAttribute("aria-expanded", "false");
    unmount();
    render(view(), { store });
    expect(screen.getAllByTestId("transcript-toggle")[0]).toHaveAttribute(
      "aria-expanded",
      "false",
    );
    expect(preferences.get("agent_diff:diff-call")).toBe(false);
  });
});
