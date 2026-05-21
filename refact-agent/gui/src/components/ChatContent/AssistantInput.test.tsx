import { beforeEach, describe, expect, test, vi } from "vitest";
const mermaidMock = vi.hoisted(() => ({
  render: vi.fn(() =>
    Promise.resolve({
      svg: '<svg viewBox="0 0 10 10" width="10" height="10"></svg>',
    }),
  ),
  initialize: vi.fn(),
}));

vi.mock("mermaid", () => ({
  default: mermaidMock,
}));

vi.mock("../../features/Buddy/reportBuddyFrontendError", async () => {
  const actual = await vi.importActual<
    typeof import("../../features/Buddy/reportBuddyFrontendError")
  >("../../features/Buddy/reportBuddyFrontendError");
  return {
    ...actual,
    reportBuddyFrontendError: vi.fn(),
  };
});

import { render, screen, waitFor } from "../../utils/test-utils";
import { AssistantInput } from "./AssistantInput";
import type { DiffChunk, ToolCall } from "../../services/refact/types";

describe("AssistantInput", () => {
  beforeEach(() => {
    mermaidMock.render.mockClear();
  });

  test("renders streaming message content as markdown immediately", () => {
    const { rerender } = render(
      <AssistantInput message="## Streaming title" isStreaming />,
    );

    expect(
      screen.getByRole("heading", { name: "Streaming title" }),
    ).toBeInTheDocument();

    rerender(<AssistantInput message="## Streaming title" />);

    expect(
      screen.getByRole("heading", { name: "Streaming title" }),
    ).toBeInTheDocument();
  });

  test("keeps incomplete streaming mermaid fence as raw code until the fence closes", async () => {
    const { rerender } = render(
      <AssistantInput
        message={"```mermaid\nflowchart LR\nA --> B"}
        isStreaming
      />,
    );

    expect(screen.getByText(/flowchart LR/)).toBeInTheDocument();
    expect(mermaidMock.render).not.toHaveBeenCalled();
    expect(screen.queryByText("Rendering…")).not.toBeInTheDocument();

    rerender(
      <AssistantInput
        message={"```mermaid\nflowchart LR\nA --> B\n```"}
        isStreaming
      />,
    );

    expect(screen.getByText(/flowchart LR/)).toBeInTheDocument();
    expect(mermaidMock.render).not.toHaveBeenCalled();

    rerender(
      <AssistantInput message={"```mermaid\nflowchart LR\nA --> B\n```"} />,
    );

    await waitFor(() => expect(mermaidMock.render).toHaveBeenCalledTimes(1));
  });

  test("keeps incomplete streaming html fence as raw code until the fence closes", () => {
    const { rerender } = render(
      <AssistantInput message={"```html\n<div>hello"} isStreaming />,
    );

    expect(screen.getByText(/<div>hello/)).toBeInTheDocument();
    expect(screen.queryByTitle("HTML Preview")).not.toBeInTheDocument();

    rerender(
      <AssistantInput message={"```html\n<div>hello</div>\n```"} isStreaming />,
    );

    expect(screen.getByText(/<div>hello<\/div>/)).toBeInTheDocument();
    expect(screen.queryByTitle("HTML Preview")).not.toBeInTheDocument();

    rerender(<AssistantInput message={"```html\n<div>hello</div>\n```"} />);

    expect(screen.getByTitle("HTML Preview")).toBeInTheDocument();
  });

  test("passes diffsByToolId through to serverExecutedTools so diff blocks are not repeated outside the tool card", () => {
    const toolCall: ToolCall = {
      id: "call-1",
      index: 0,
      function: {
        name: "apply_patch",
        arguments: "{}",
      },
    };

    const diffChunk: DiffChunk = {
      file_name: "debug_codex_models.py",
      file_action: "edit",
      line1: 10,
      line2: 11,
      lines_remove: "old line",
      lines_add: "new line",
    };

    render(
      <AssistantInput
        message="I'll update the debug script."
        serverExecutedTools={[toolCall]}
        diffsByToolId={{ "call-1": [diffChunk] }}
      />,
    );

    expect(screen.getAllByText(/debug_codex_models\.py/i)).toHaveLength(1);
    expect(screen.queryByText(/Tasks 0\/3/i)).not.toBeInTheDocument();
  });
});
