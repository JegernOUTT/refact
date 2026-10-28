import { describe, expect, it } from "vitest";
import { http, HttpResponse } from "msw";
import { render, screen } from "../../../utils/test-utils";
import type { DiffChunk, ToolCall } from "../../../services/refact/types";
import { EditTool } from "./EditTool";
import { editToolFilePaths } from "./DiffBlockStats";
import { server } from "../../../utils/mockServer";

function makeToolCall(id: string): ToolCall {
  return {
    id,
    index: 0,
    function: {
      name: "update_textdoc",
      arguments: JSON.stringify({
        path: "src/demo.ts",
        old_str: "old",
        replacement: "new",
      }),
    },
  };
}

server.use(
  http.post("*/v1/fullpath", async ({ request }) => {
    const body = (await request.json()) as { path?: string };
    return HttpResponse.json({
      fullpath: body.path ?? "",
      is_directory: false,
    });
  }),
);

const ideConfig = {
  host: "vscode" as const,
  lspPort: 8001,
  themeProps: { appearance: "dark" as const },
};

describe("EditTool", () => {
  it("renders edit hunks expanded by default with one line-number column", () => {
    const diff: DiffChunk = {
      file_name: "src/demo.ts",
      file_action: "edit",
      line1: 10,
      line2: 20,
      lines_remove: "before\nold\nafter\n",
      lines_add: "before\nnew\nafter\n",
    };

    const { container } = render(
      <EditTool
        toolCall={makeToolCall("edit-1")}
        diffs={[diff]}
        isActiveTool={false}
      />,
    );

    expect(screen.getByText("@@ -10,3 +20,3 @@")).toBeInTheDocument();
    expect(screen.getByText("old")).toBeInTheDocument();
    expect(screen.getByText("new")).toBeInTheDocument();

    const lineNumbers = Array.from(
      container.querySelectorAll('span[class*="lineNumber"]'),
    ).map((node) => node.textContent);
    expect(lineNumbers).toEqual(["20", "11", "21", "22"]);
    expect(container.querySelector('[class*="oldLineNumber"]')).toBeNull();
    expect(container.querySelector('[class*="newLineNumber"]')).toBeNull();
  });

  it("prefers backend-provided context lines around edit hunks", () => {
    const diff: DiffChunk = {
      file_name: "src/backend.ts",
      file_action: "edit",
      line1: 20,
      line2: 20,
      lines_before: "previous line\n",
      lines_remove: "old line\n",
      lines_add: "new line\n",
      lines_after: "next line\n",
    };

    const { container } = render(
      <EditTool
        toolCall={makeToolCall("edit-context")}
        diffs={[diff]}
        isActiveTool={false}
      />,
    );

    expect(screen.getByText("previous line")).toBeInTheDocument();
    expect(screen.getByText("old line")).toBeInTheDocument();
    expect(screen.getByText("new line")).toBeInTheDocument();
    expect(screen.getByText("next line")).toBeInTheDocument();

    const lineNumbers = Array.from(
      container.querySelectorAll('span[class*="lineNumber"]'),
    ).map((node) => node.textContent);
    expect(lineNumbers).toEqual(["19", "20", "20", "21"]);
  });

  it("renders multi-file edit chunks inline", () => {
    const diffs: DiffChunk[] = [
      {
        file_name: "src/one.ts",
        file_action: "edit",
        line1: 1,
        line2: 1,
        lines_remove: "old-one\n",
        lines_add: "new-one\n",
      },
      {
        file_name: "src/two.ts",
        file_action: "edit",
        line1: 1,
        line2: 1,
        lines_remove: "old-two\n",
        lines_add: "new-two\n",
      },
    ];

    render(
      <EditTool
        toolCall={makeToolCall("edit-multi")}
        diffs={diffs}
        isActiveTool={false}
      />,
    );

    expect(screen.getByRole("button", { name: "one.ts" })).toBeInTheDocument();
    expect(screen.getByText("old-one")).toBeInTheDocument();
    expect(screen.getByText("old-two")).toBeInTheDocument();
  });

  it("keeps actions available for multi-file edit chunks", () => {
    const diffs: DiffChunk[] = [
      {
        file_name: "src/one.ts",
        file_action: "edit",
        line1: 1,
        line2: 1,
        lines_remove: "old-one\n",
        lines_add: "new-one\n",
      },
      {
        file_name: "src/two.ts",
        file_action: "edit",
        line1: 1,
        line2: 1,
        lines_remove: "old-two\n",
        lines_add: "new-two\n",
      },
    ];

    render(
      <EditTool
        toolCall={makeToolCall("edit-multi-actions")}
        diffs={diffs}
        isActiveTool={false}
      />,
      { preloadedState: { config: ideConfig } },
    );

    expect(screen.getAllByRole("button", { name: "Apply diff" })).toHaveLength(
      2,
    );
  });

  it("hides Apply diff actions when the host lacks ideDiffPasteBack", () => {
    const diff: DiffChunk = {
      file_name: "src/web.ts",
      file_action: "edit",
      line1: 1,
      line2: 1,
      lines_remove: "old-web\n",
      lines_add: "new-web\n",
    };

    render(
      <EditTool
        toolCall={makeToolCall("edit-web")}
        diffs={[diff]}
        isActiveTool={false}
      />,
    );

    expect(screen.getByText("old-web")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Apply diff" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Replace selection" }),
    ).not.toBeInTheDocument();
  });

  it("renders a large hunk in full inside the transcript clamp", () => {
    const diff: DiffChunk = {
      file_name: "src/large.ts",
      file_action: "edit",
      line1: 1,
      line2: 1,
      lines_remove: "",
      lines_add: Array.from({ length: 245 }, (_, i) => `new-${i + 1}`).join(
        "\n",
      ),
    };

    render(
      <EditTool
        toolCall={makeToolCall("edit-2")}
        diffs={[diff]}
        isActiveTool={false}
      />,
    );

    expect(screen.getByText("new-245")).toBeInTheDocument();
    expect(screen.getByTestId("transcript-clamp")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /more diff lines/i }),
    ).toBeNull();
  });

  it("shows an open unified row with verb, file target and diff stats", () => {
    const diff: DiffChunk = {
      file_name: "src/demo.ts",
      file_action: "edit",
      line1: 1,
      line2: 1,
      lines_remove: "a\nb\n",
      lines_add: "c\n",
    };

    render(
      <EditTool
        toolCall={makeToolCall("edit-row")}
        diffs={[diff]}
        isActiveTool={false}
      />,
    );

    expect(screen.getByTestId("transcript-verb")).toHaveTextContent("Edit");
    expect(screen.getByTestId("transcript-target")).toHaveTextContent(
      "demo.ts",
    );
    expect(screen.getByTestId("transcript-meta")).toHaveTextContent("+1 −2");
    expect(screen.getByTestId("transcript-toggle")).toHaveAttribute(
      "aria-expanded",
      "true",
    );
  });

  it("names apply_patch targets from the patch headers", () => {
    const patch = [
      "*** Begin Patch",
      "*** Update File: src/a.ts",
      "*** Add File: src/b.ts",
      "+++ b/src/c.ts",
      "+++ /dev/null",
      "*** End Patch",
    ].join("\n");

    expect(editToolFilePaths(JSON.stringify({ patch }))).toEqual([
      "src/a.ts",
      "src/b.ts",
      "src/c.ts",
    ]);
    expect(editToolFilePaths(JSON.stringify({ path: "src/x.ts" }))).toEqual([
      "src/x.ts",
    ]);
    expect(editToolFilePaths("not json")).toEqual([]);
  });

  it("labels created files with the Create verb", () => {
    const diff: DiffChunk = {
      file_name: "src/fresh.ts",
      file_action: "new",
      line1: 1,
      line2: 1,
      lines_remove: "",
      lines_add: "hello\n",
    };

    render(
      <EditTool
        toolCall={makeToolCall("edit-create")}
        diffs={[diff]}
        isActiveTool={false}
      />,
    );

    expect(screen.getByTestId("transcript-verb")).toHaveTextContent("Create");
  });

  it("does not show a large-diff cap at the visible-line boundary", () => {
    const diff: DiffChunk = {
      file_name: "src/boundary.ts",
      file_action: "edit",
      line1: 1,
      line2: 1,
      lines_remove: "",
      lines_add: Array.from({ length: 240 }, (_, i) => `new-${i + 1}`).join(
        "\n",
      ),
    };

    render(
      <EditTool
        toolCall={makeToolCall("edit-boundary")}
        diffs={[diff]}
        isActiveTool={false}
      />,
    );

    expect(screen.getByText("new-240")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /more diff lines/i }),
    ).toBeNull();
  });
});
