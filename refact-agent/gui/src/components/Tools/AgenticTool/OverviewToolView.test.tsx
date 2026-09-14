import { describe, expect, test } from "vitest";
import { render, screen } from "../../../utils/test-utils";
import { OverviewToolView } from "./OverviewToolView";
import { familyModel } from "./familyFixtures";
import { MAX_AGENT_ROWS } from "./FamilyToolCard";
import { PAGE_CHARS } from "./PagedText";

function overviewOutput(count: number): string {
  const rows = Array.from(
    { length: count },
    (_value, index) =>
      `- agent-${index + 1} — Task ${index + 1} — running — step ${index + 1}`,
  );
  return ["# Agents Overview", "", ...rows].join("\n");
}

function renderOverview(input: Parameters<typeof familyModel>[0]) {
  return render(
    <OverviewToolView
      model={familyModel(input)}
      content={input.content ?? null}
    />,
  );
}

describe("OverviewToolView", () => {
  test("renders a structured roster", () => {
    renderOverview({ toolName: "agents_overview", content: overviewOutput(2) });
    expect(screen.getByLabelText("Agents overview")).toBeInTheDocument();
    expect(screen.getByTestId("agentic-tool-agents")).toBeInTheDocument();
    expect(screen.getByText("Task 1")).toBeInTheDocument();
  });

  test("reports an empty legacy roster without crashing", () => {
    renderOverview({ toolName: "agent_list", content: "# Background Agents" });
    expect(
      screen.getByText("No agents were listed in this snapshot."),
    ).toBeInTheDocument();
  });

  test("flags an unrecognised response and still pages it", () => {
    renderOverview({
      toolName: "task_overview",
      content: "z".repeat(PAGE_CHARS + 3),
    });
    expect(screen.getByTestId("agentic-tool-legacy")).toBeInTheDocument();
    expect(screen.getByTestId("agentic-tool-body").textContent).toHaveLength(
      PAGE_CHARS,
    );
  });

  test("renders an explicit malformed state instead of raw payload", () => {
    renderOverview({ toolName: "agent_list", content: '[{"id":"a"' });
    expect(screen.getByRole("alert")).toHaveTextContent("could not be read");
    expect(screen.queryByTestId("agentic-tool-agents")).toBeNull();
  });

  test("keeps failure and streaming states distinct", () => {
    const view = renderOverview({
      toolName: "agents_overview",
      content: "Coordination is disabled",
      toolFailed: true,
    });
    expect(screen.getByTestId("agentic-tool-error")).toHaveTextContent(
      "disabled",
    );
    view.rerender(
      <OverviewToolView
        model={familyModel({
          toolName: "agents_overview",
          hasResult: false,
          isBusy: true,
        })}
        content={null}
      />,
    );
    expect(screen.getByRole("status")).toHaveTextContent(
      "No roster snapshot returned yet",
    );
  });

  test("bounds long rosters", () => {
    renderOverview({
      toolName: "agents_overview",
      content: overviewOutput(10),
    });
    expect(
      screen.getByTestId("agentic-tool-agents").querySelectorAll("li"),
    ).toHaveLength(MAX_AGENT_ROWS);
    expect(
      screen.getByText(`Showing ${MAX_AGENT_ROWS} of 10 agents.`),
    ).toBeInTheDocument();
  });
});
