import { describe, expect, it } from "vitest";
import { fireEvent, render, screen } from "../../utils/test-utils";
import { parseAgentDiffOutput } from "./AgentDiffModel";
import { AgentDiffContent } from "./AgentDiffView";

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
    const report = parseAgentDiffOutput(DIFF_OUTPUT);
    if (!report) throw new Error("expected diff report");

    render(<AgentDiffContent report={report} />);

    fireEvent.click(
      screen.getAllByText("src/components/ChatContent/AgentPulseView.tsx")[0],
    );

    expect(
      screen.getByText("export const AgentPulseView = true;"),
    ).toBeInTheDocument();
    expect(screen.queryByText("new line")).not.toBeInTheDocument();
  });
});
