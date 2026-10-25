import { describe, expect, it } from "vitest";
import { act as rtlAct } from "@testing-library/react";
import {
  createDefaultChatState,
  fireEvent,
  render,
  screen,
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
    content === null ? [] : [diffMessage(content, toolFailed)];
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

    expect(screen.getByText("Agent diff: T-29")).toBeInTheDocument();
    expect(screen.getByText("refact/task/T-29-agent")).toBeInTheDocument();
    expect(screen.getByText("commit abc123")).toBeInTheDocument();
    expect(screen.getByText("All files")).toBeInTheDocument();
    expect(
      screen.getAllByText("src/tools/tool_task_check_agents.rs").length,
    ).toBeGreaterThan(0);
    expect(
      screen.getAllByText("src/components/ChatContent/AgentPulseView.tsx")
        .length,
    ).toBeGreaterThan(0);
    expect(
      screen.getByText(
        "... (4 more lines, use mode='name-only' to see all files)",
      ),
    ).toBeInTheDocument();
  });

  it("filters rendered unified diff by selected file", () => {
    renderDiff(DIFF_OUTPUT);
    const outer = screen.getByRole("button", {
      name: "2 changed files shown · check-agents redesign Partial diff",
    });
    fireEvent.click(outer);
    const details = screen.getByRole("button", { name: "More details" });
    fireEvent.click(details);

    fireEvent.click(
      screen.getAllByText("src/components/ChatContent/AgentPulseView.tsx")[0],
    );

    expect(
      screen.getByText("export const AgentPulseView = true;"),
    ).toBeInTheDocument();
    expect(screen.queryByText("new line")).not.toBeInTheDocument();
    expect(outer).toHaveAttribute("aria-expanded", "true");
    expect(details).toHaveAttribute("aria-expanded", "true");
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

describe("AgentDiffView compact route", () => {
  it("starts closed with observed changed files and partial coverage, preserving hidden file controls", async () => {
    const { container, user } = renderDiff(DIFF_OUTPUT);
    const outer = screen.getByRole("button", {
      name: "2 changed files shown · check-agents redesign Partial diff",
    });
    expect(outer).toHaveAttribute("aria-expanded", "false");
    expect(outer.closest("section")).toHaveAttribute("data-density", "compact");
    expect(outer).not.toHaveTextContent("T-29");
    expect(screen.queryByText("All files")).not.toBeInTheDocument();
    await user.click(outer);
    expect(container.querySelectorAll("dt")).toHaveLength(3);
    expect(container.querySelector("dd")?.textContent).toBe("2");
    const details = screen.getByRole("button", { name: "More details" });
    const files = screen.getByRole("button", { name: "All files" });
    expect(details).toHaveAttribute("aria-expanded", "false");
    expect(files.closest("[inert]")).not.toBeNull();
    await user.click(details);
    expect(files.closest("[inert]")).toBeNull();
    expect(screen.getByText("refact/task/T-29-agent")).toBeInTheDocument();
    await user.click(outer);
    expect(files.closest("[inert]")).not.toBeNull();
    outer.focus();
    await user.tab();
    expect(files).not.toHaveFocus();
  });

  it.each([
    [null, false, false, "Inspect changed files Diff unknown", "idle"],
    [null, false, true, "Inspect changed files Reading diff", "running"],
    ["not a diff", false, false, "Inspect changed files Diff unknown", "idle"],
    [
      'agent_diff(card_id="T-29")',
      true,
      false,
      "Inspect changed files Request failed",
      "error",
    ],
    [
      `# Agent Diff for T-29\n${FENCE}diff\n${FENCE}`,
      false,
      false,
      "Inspect changed files Diff unknown",
      "idle",
    ],
    [
      `# Agent Diff for T-29\n${FENCE}\n(no changes detected)\n${FENCE}`,
      false,
      false,
      "0 changed files shown Diff received",
      "idle",
    ],
  ])(
    "does not invent changes or request success: %s",
    (content, failed, waiting, name, status) => {
      renderDiff(content, failed, waiting);
      const outer = screen.getByRole("button", { name });
      expect(outer).toHaveAttribute("aria-expanded", "false");
      expect(outer.closest("section")).toHaveAttribute("data-status", status);
    },
  );

  it("honors stored open and closed choices across state changes and remounts", () => {
    const preferences = new Map([["tc:diff-call", true]]);
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
    const outer = screen.getByRole("button", {
      name: "2 changed files shown · check-agents redesign Partial diff",
    });
    const sibling = screen.getByRole("button", {
      name: "Inspect changed files Diff unknown",
    });
    const details = screen.getByRole("button", { name: "More details" });
    fireEvent.click(details);
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
    expect(details).toHaveAttribute("aria-expanded", "true");
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
    expect(
      screen.getByRole("button", {
        name: "Inspect changed files Request failed",
      }),
    ).toHaveAttribute("aria-expanded", "false");
  });
});
