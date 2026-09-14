import { describe, expect, test } from "vitest";
import { render, screen } from "../../../utils/test-utils";
import { MemoryToolView } from "./MemoryToolView";
import { familyModel } from "./familyFixtures";
import { REPORT_MAX_FIELDS } from "./familyReport";
import { PAGE_CHARS } from "./PagedText";

function jsonPayload(count: number): string {
  const payload: Record<string, string> = {};
  for (let index = 0; index < count; index += 1) {
    payload[`field_${index + 1}`] = `value ${index + 1}`;
  }
  return JSON.stringify(payload);
}

function renderMemory(input: Parameters<typeof familyModel>[0]) {
  return render(
    <MemoryToolView
      model={familyModel(input)}
      content={input.content ?? null}
    />,
  );
}

describe("MemoryToolView", () => {
  test("renders json response fields instead of raw json", () => {
    const content = JSON.stringify({ slug: "locks", kind: "gotcha" });
    renderMemory({ toolName: "task_mem_save", content });
    expect(screen.getByLabelText("Task memory receipt")).toBeInTheDocument();
    expect(screen.getByLabelText("Response fields")).toHaveTextContent("locks");
    expect(screen.queryByTestId("agentic-tool-body")).toBeNull();
  });

  test("accepts the legacy markdown memory shape", () => {
    renderMemory({
      toolName: "task_mem_search",
      args: { query: "locks" },
      content: "# Task memories\n- Kind: gotcha\n- Namespace: task",
    });
    expect(screen.getByLabelText("Memory facts")).toHaveTextContent(
      "Namespace",
    );
  });

  test("flags an unrecognised response and still pages it", () => {
    renderMemory({
      toolName: "task_mem_get",
      content: "k".repeat(PAGE_CHARS + 4),
    });
    expect(screen.getByTestId("agentic-tool-legacy")).toBeInTheDocument();
    expect(screen.getByTestId("agentic-tool-body").textContent).toHaveLength(
      PAGE_CHARS,
    );
  });

  test("renders an explicit malformed state instead of raw payload", () => {
    renderMemory({ toolName: "task_mem_inbox", content: '{"memories":[' });
    expect(screen.getByRole("alert")).toHaveTextContent("could not be read");
    expect(screen.queryByLabelText("Response fields")).toBeNull();
  });

  test("keeps failure and streaming states distinct", () => {
    const view = renderMemory({
      toolName: "task_mem_pin",
      content: "Memory not found",
      toolFailed: true,
    });
    expect(screen.getByTestId("agentic-tool-error")).toHaveTextContent(
      "not found",
    );
    view.rerender(
      <MemoryToolView
        model={familyModel({
          toolName: "task_mem_save",
          hasResult: false,
          isBusy: true,
        })}
        content={null}
      />,
    );
    expect(screen.getByRole("status")).toHaveTextContent(
      "No memory outcome confirmed yet",
    );
  });

  test("bounds wide json payloads", () => {
    renderMemory({ toolName: "task_mem_search", content: jsonPayload(15) });
    expect(
      screen.getByLabelText("Response fields").querySelectorAll("dt"),
    ).toHaveLength(REPORT_MAX_FIELDS);
    expect(
      screen.getByText(`Showing ${REPORT_MAX_FIELDS} of 15 entries.`),
    ).toBeInTheDocument();
  });
});
