import { describe, expect, test } from "vitest";
import { render, screen } from "../../../utils/test-utils";
import { InspectToolView } from "./InspectToolView";
import { familyModel } from "./familyFixtures";
import { MAX_DETAIL_ROWS } from "./FamilyToolCard";
import { PAGE_CHARS } from "./PagedText";

const STATUS = [
  "# Subagent: Ship it",
  "- Status: running",
  "- Agent ID: bgagent-7",
  "- Step count: 12",
  "",
  "Child trajectory: [view](refact://chat/child-9)",
].join("\n");

function renderInspect(input: Parameters<typeof familyModel>[0]) {
  return render(
    <InspectToolView
      model={familyModel(input)}
      content={input.content ?? null}
    />,
  );
}

describe("InspectToolView", () => {
  test("renders observed facts as a description list", () => {
    renderInspect({
      toolName: "agent_status",
      args: { agent_id: "bgagent-7" },
      content: STATUS,
    });
    expect(screen.getByLabelText("Agent inspection")).toBeInTheDocument();
    expect(screen.getByLabelText("Observed facts")).toHaveTextContent(
      "Step count",
    );
    expect(screen.getByTestId("agentic-tool-child-note")).toHaveTextContent(
      "child-9",
    );
  });

  test("accepts the legacy one-line status shape", () => {
    renderInspect({
      toolName: "agent_wait",
      content: "Agent has not finished yet. Current status: running.",
    });
    expect(screen.getByTestId("agentic-tool-summary")).toHaveTextContent(
      "has not finished yet",
    );
  });

  test("flags an unrecognised response and still pages it", () => {
    renderInspect({
      toolName: "agent_result",
      content: "q".repeat(PAGE_CHARS + 7),
    });
    expect(screen.getByTestId("agentic-tool-legacy")).toBeInTheDocument();
    expect(screen.getByTestId("agentic-tool-body").textContent).toHaveLength(
      PAGE_CHARS,
    );
  });

  test("renders an explicit malformed state instead of raw payload", () => {
    renderInspect({ toolName: "agent_result", content: '{"status":' });
    expect(screen.getByRole("alert")).toHaveTextContent("could not be read");
    expect(screen.queryByTestId("agentic-tool-summary")).toBeNull();
  });

  test("keeps failure and streaming states distinct", () => {
    const view = renderInspect({
      toolName: "agent_result",
      content: "Background agent not found.",
      toolFailed: true,
    });
    expect(screen.getByTestId("agentic-tool-error")).toHaveTextContent(
      "not found",
    );
    view.rerender(
      <InspectToolView
        model={familyModel({
          toolName: "agent_wait",
          hasResult: false,
          isBusy: true,
        })}
        content={null}
      />,
    );
    expect(screen.getByRole("status")).toHaveTextContent(
      "No observation returned yet",
    );
  });

  test("bounds long observation fact lists", () => {
    const lines = Array.from(
      { length: 15 },
      (_value, index) => `- Fact ${index + 1}: value ${index + 1}`,
    );
    renderInspect({
      toolName: "agent_status",
      content: ["# Subagent: Ship it", ...lines].join("\n"),
    });
    expect(
      screen.getByLabelText("Observed facts").querySelectorAll("dt"),
    ).toHaveLength(MAX_DETAIL_ROWS);
    expect(
      screen.getByText(`Showing ${MAX_DETAIL_ROWS} of 15 entries.`),
    ).toBeInTheDocument();
  });
});
