import { describe, expect, it } from "vitest";
import { http, HttpResponse } from "msw";
import { render, screen } from "../../../utils/test-utils";
import type { DiffChunk, ToolCall } from "../../../services/refact/types";
import { EditTool } from "./EditTool";
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
    );

    expect(screen.getAllByRole("button", { name: "Apply diff" })).toHaveLength(
      2,
    );
  });

  it("caps very large edit hunks behind a show-more control", async () => {
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

    const { user } = render(
      <EditTool
        toolCall={makeToolCall("edit-2")}
        diffs={[diff]}
        isActiveTool={false}
      />,
    );

    expect(screen.getByText("new-240")).toBeInTheDocument();
    expect(screen.queryByText("new-245")).not.toBeInTheDocument();

    await user.click(
      screen.getByRole("button", { name: "Show 5 more diff lines" }),
    );

    expect(screen.getByText("new-245")).toBeInTheDocument();
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
