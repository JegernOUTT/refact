import { describe, expect, test } from "vitest";
import { render, screen } from "../../../utils/test-utils";
import { BoardToolView } from "./BoardToolView";
import { familyModel, markdownRows } from "./familyFixtures";
import { PAGE_CHARS } from "./PagedText";
import { REPORT_MAX_ROWS } from "./familyReport";

function renderBoard(input: Parameters<typeof familyModel>[0]) {
  return render(
    <BoardToolView
      model={familyModel(input)}
      content={input.content ?? null}
    />,
  );
}

describe("BoardToolView", () => {
  test("renders a structured board receipt", () => {
    const content = markdownRows(1);
    renderBoard({ toolName: "board_get", content });
    expect(screen.getByLabelText("Task board receipt")).toBeInTheDocument();
    expect(screen.getByRole("columnheader", { name: "Card" })).toBeVisible();
    expect(screen.getByRole("cell", { name: "Card 1" })).toBeVisible();
    expect(screen.queryByTestId("agentic-tool-legacy")).toBeNull();
  });

  test("accepts the legacy one-line confirmation shape", () => {
    const content = "Created card T-5 in Planned column";
    renderBoard({
      toolName: "board_create",
      args: { card_id: "T-5" },
      content,
    });
    expect(screen.getByTestId("agentic-tool-summary")).toHaveTextContent(
      "Created card T-5 in Planned column",
    );
    expect(screen.queryByTestId("agentic-tool-legacy")).toBeNull();
  });

  test("flags an unrecognised response and still pages it", () => {
    const content = "x".repeat(PAGE_CHARS * 2 + 10);
    renderBoard({ toolName: "task_verify_card", content });
    expect(screen.getByTestId("agentic-tool-legacy")).toBeInTheDocument();
    expect(screen.getByTestId("agentic-tool-body").textContent).toHaveLength(
      PAGE_CHARS,
    );
  });

  test("renders an explicit malformed state instead of raw payload", () => {
    const content = '{"card_id": "T-1"';
    renderBoard({ toolName: "board_update", content });
    expect(screen.getByRole("alert")).toHaveTextContent("could not be read");
    expect(screen.queryByTestId("agentic-tool-body")).toBeNull();
    expect(
      screen.getByTestId("agentic-tool-raw-response").closest("details"),
    ).not.toBeNull();
  });

  test("keeps failure and streaming states distinct", () => {
    const view = renderBoard({
      toolName: "board_move",
      content: "Card T-9 not found",
      toolFailed: true,
    });
    expect(screen.getByTestId("agentic-tool-error")).toHaveTextContent(
      "not found",
    );
    view.rerender(
      <BoardToolView
        model={familyModel({
          toolName: "board_move",
          hasResult: false,
          isBusy: true,
        })}
        content={null}
      />,
    );
    expect(screen.getByRole("status")).toHaveTextContent(
      "No board outcome confirmed yet",
    );
  });

  test("bounds long board tables", () => {
    const content = markdownRows(20);
    renderBoard({ toolName: "ready_cards", content });
    expect(screen.getAllByRole("row")).toHaveLength(REPORT_MAX_ROWS + 1);
    expect(
      screen.getByText(`Showing ${REPORT_MAX_ROWS} of 20 rows.`),
    ).toBeInTheDocument();
  });
});
