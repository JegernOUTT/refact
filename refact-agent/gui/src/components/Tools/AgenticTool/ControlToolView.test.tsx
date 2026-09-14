import { describe, expect, test } from "vitest";
import { render, screen } from "../../../utils/test-utils";
import { ControlToolView } from "./ControlToolView";
import { familyModel } from "./familyFixtures";
import { REPORT_MAX_ITEMS } from "./familyReport";
import { PAGE_CHARS } from "./PagedText";

function renderControl(input: Parameters<typeof familyModel>[0]) {
  return render(
    <ControlToolView
      model={familyModel(input)}
      content={input.content ?? null}
    />,
  );
}

describe("ControlToolView", () => {
  test("renders an accepted control request", () => {
    renderControl({
      toolName: "resume_agent",
      args: { card_id: "T-1" },
      content: "Resumed T-1.",
    });
    expect(screen.getByLabelText("Agent control receipt")).toBeInTheDocument();
    expect(screen.getByTestId("agentic-tool-summary")).toHaveTextContent(
      "Resumed T-1.",
    );
    expect(
      screen.getByTestId("agentic-tool-raw-args").closest("details")?.open,
    ).toBe(false);
  });

  test("accepts the legacy control acknowledgement shape", () => {
    renderControl({
      toolName: "agent_cancel",
      args: { agent_id: "abc", reason: "stale" },
      content: "✓ Cancel requested for: abc.",
    });
    expect(screen.getByTestId("agentic-tool-summary")).toHaveTextContent(
      "Cancel requested",
    );
  });

  test("flags an unrecognised response and still pages it", () => {
    renderControl({
      toolName: "merge_agent",
      content: "m".repeat(PAGE_CHARS + 11),
    });
    expect(screen.getByTestId("agentic-tool-legacy")).toBeInTheDocument();
    expect(screen.getByTestId("agentic-tool-body").textContent).toHaveLength(
      PAGE_CHARS,
    );
  });

  test("renders an explicit malformed state instead of raw payload", () => {
    renderControl({
      toolName: "pause_agent",
      argsRaw: "[1,2]",
      content: "Paused T-1.",
    });
    expect(screen.getByRole("alert")).toHaveTextContent("could not be read");
    expect(screen.queryByTestId("agentic-tool-summary")).toBeNull();
  });

  test("keeps failure and streaming states distinct", () => {
    const view = renderControl({
      toolName: "cancel_agent",
      content: "Card T-9 not found",
      toolFailed: true,
    });
    expect(screen.getByTestId("agentic-tool-error")).toHaveTextContent(
      "not found",
    );
    view.rerender(
      <ControlToolView
        model={familyModel({
          toolName: "merge_ready_in_order",
          hasResult: false,
          isBusy: true,
        })}
        content={null}
      />,
    );
    expect(screen.getByRole("status")).toHaveTextContent(
      "No control outcome confirmed yet",
    );
  });

  test("bounds long affected-agent lists", () => {
    const items = Array.from(
      { length: 12 },
      (_value, index) => `- T-${index + 1} merged cleanly`,
    );
    renderControl({
      toolName: "merge_ready_in_order",
      content: ["# Merge run", ...items].join("\n"),
    });
    expect(
      screen.getByTestId("agentic-tool-items").querySelectorAll("li"),
    ).toHaveLength(REPORT_MAX_ITEMS);
    expect(
      screen.getByText(`Showing ${REPORT_MAX_ITEMS} of 12 entries.`),
    ).toBeInTheDocument();
  });
});
