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

type MermaidInitializeConfig = {
  themeVariables?: Record<string, string>;
};

describe("AssistantInput", () => {
  beforeEach(() => {
    mermaidMock.render.mockClear();
    mermaidMock.initialize.mockClear();
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
    expect(mermaidMock.initialize).toHaveBeenCalled();

    const initializeConfig = mermaidMock.initialize.mock.calls.at(-1)?.[0] as
      | MermaidInitializeConfig
      | undefined;
    const themeVariables = initializeConfig?.themeVariables;

    expect(themeVariables).toBeDefined();
    expect(JSON.stringify(themeVariables)).not.toContain("var(");
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

  test("does not mount the HTML preview card at all while streaming", () => {
    const { rerender } = render(
      <AssistantInput message={"```html\n<div>hello</div>\n```"} isStreaming />,
    );

    // No preview card chrome mid-stream — the fence renders as plain code, so
    // there are no disabled-looking buttons and no remount when the iframe
    // appears at stream end.
    expect(screen.queryByText("HTML Preview")).not.toBeInTheDocument();
    expect(screen.getByText(/<div>hello<\/div>/)).toBeInTheDocument();

    rerender(<AssistantInput message={"```html\n<div>hello</div>\n```"} />);

    expect(screen.getByText("HTML Preview")).toBeInTheDocument();
    expect(screen.getByTitle("HTML Preview")).toBeInTheDocument();
  });

  test("dispatches uppercase fence languages to the special renderers", async () => {
    render(
      <AssistantInput message={"```MERMAID\nflowchart LR\nA --> B\n```"} />,
    );

    await waitFor(() => expect(mermaidMock.render).toHaveBeenCalledTimes(1));
  });

  test("renders Claude Code augmented tool aliases as their specialized tool cards", () => {
    const aliasCalls: ToolCall[] = [
      {
        id: "alias-read",
        index: 0,
        function: {
          name: "t_cat",
          arguments: JSON.stringify({ paths: "src/main.rs" }),
        },
      },
      {
        id: "alias-grep",
        index: 1,
        function: {
          name: "t_regex_search",
          arguments: JSON.stringify({ pattern: "TODO", scope: "workspace" }),
        },
      },
      {
        id: "alias-plan",
        index: 2,
        function: {
          name: "t_plan",
          arguments: "{}",
        },
      },
      {
        id: "alias-web-search",
        index: 3,
        function: {
          name: "WebSearch",
          arguments: JSON.stringify({ query: "refact" }),
        },
      },
    ];

    render(<AssistantInput message="" serverExecutedTools={aliasCalls} />);

    expect(screen.getByText(/Read/i)).toBeInTheDocument();
    expect(screen.getByText("TODO")).toBeInTheDocument();
    expect(screen.getByText(/Plan solution/i)).toBeInTheDocument();
    expect(screen.getByText(/Search web/i)).toBeInTheDocument();
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
