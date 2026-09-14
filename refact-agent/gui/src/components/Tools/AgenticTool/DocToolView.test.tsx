import { describe, expect, test } from "vitest";
import { render, screen } from "../../../utils/test-utils";
import { DocToolView } from "./DocToolView";
import { familyModel } from "./familyFixtures";
import { REPORT_MAX_ROWS } from "./familyReport";
import { PAGE_CHARS } from "./PagedText";

function docTable(count: number): string {
  const rows = Array.from(
    { length: count },
    (_value, index) => `| doc-${index + 1} | Design ${index + 1} | spec |`,
  );
  return [
    "# Task documents",
    "",
    "| Slug | Name | Kind |",
    "|------|------|------|",
    ...rows,
  ].join("\n");
}

function renderDoc(input: Parameters<typeof familyModel>[0]) {
  return render(
    <DocToolView model={familyModel(input)} content={input.content ?? null} />,
  );
}

describe("DocToolView", () => {
  test("renders a structured document receipt", () => {
    renderDoc({ toolName: "doc_history", content: docTable(1) });
    expect(screen.getByLabelText("Task document receipt")).toBeInTheDocument();
    expect(screen.getByRole("columnheader", { name: "Slug" })).toBeVisible();
    expect(screen.getByRole("cell", { name: "Design 1" })).toBeVisible();
  });

  test("accepts the legacy one-line document confirmation", () => {
    renderDoc({
      toolName: "doc_create",
      args: { slug: "master-plan" },
      content: "Created document master-plan (version 1).",
    });
    expect(screen.getByTestId("agentic-tool-summary")).toHaveTextContent(
      "Created document master-plan",
    );
  });

  test("flags an unrecognised response and still pages it", () => {
    renderDoc({ toolName: "doc_append", content: "d".repeat(PAGE_CHARS + 2) });
    expect(screen.getByTestId("agentic-tool-legacy")).toBeInTheDocument();
    expect(screen.getByTestId("agentic-tool-body").textContent).toHaveLength(
      PAGE_CHARS,
    );
  });

  test("renders an explicit malformed state instead of raw payload", () => {
    renderDoc({ toolName: "doc_update", content: '{"slug":"a",' });
    expect(screen.getByRole("alert")).toHaveTextContent("could not be read");
    expect(screen.queryByTestId("agentic-tool-body")).toBeNull();
  });

  test("keeps failure and streaming states distinct", () => {
    const view = renderDoc({
      toolName: "doc_delete",
      content: "document `plan` does not exist",
      toolFailed: true,
    });
    expect(screen.getByTestId("agentic-tool-error")).toHaveTextContent(
      "does not exist",
    );
    view.rerender(
      <DocToolView
        model={familyModel({
          toolName: "doc_pin",
          hasResult: false,
          isBusy: true,
        })}
        content={null}
      />,
    );
    expect(screen.getByRole("status")).toHaveTextContent(
      "No document outcome confirmed yet",
    );
  });

  test("bounds long document tables", () => {
    renderDoc({ toolName: "doc_history", content: docTable(20) });
    expect(screen.getAllByRole("row")).toHaveLength(REPORT_MAX_ROWS + 1);
    expect(
      screen.getByText(`Showing ${REPORT_MAX_ROWS} of 20 rows.`),
    ).toBeInTheDocument();
  });
});
