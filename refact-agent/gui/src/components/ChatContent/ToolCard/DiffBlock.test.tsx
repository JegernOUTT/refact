import { afterEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "../../../utils/test-utils";
import type { DiffChunk } from "../../../services/refact/types";
import { DiffBlock } from "./DiffBlock";
import { EditTool } from "./EditTool";
import { FileOpTool } from "./FileOpTool";
import { diffStatsMeta, getDiffStats } from "./DiffBlockStats";
import {
  downloadOriginalDiff,
  isOversizedDiff,
  limitDiffPreview,
  MAX_DIFF_INPUT_CHARS,
  MAX_DIFF_INPUT_LINES,
  MAX_DIFF_PREVIEW_CHARS,
  MAX_DIFF_PREVIEW_LINES,
} from "./diffPreview";

vi.mock("../../../hooks/useShiki", () => ({
  useShiki: () => ({ isReady: false, highlight: vi.fn() }),
}));

function chunk(overrides: Partial<DiffChunk> = {}): DiffChunk {
  return {
    file_name: "src/demo.ts",
    file_action: "edit",
    line1: 10,
    line2: 20,
    lines_remove: "old\n",
    lines_add: "new\n",
    ...overrides,
  };
}

const fields = [
  "lines_before",
  "lines_remove",
  "lines_add",
  "lines_after",
] as const;

afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
});

describe("bounded diff preview", () => {
  it.each([
    ["huge multiline", "old\n".repeat(100_000)],
    ["line-count overflow", "\n".repeat(MAX_DIFF_INPUT_LINES + 1)],
    ["huge single line", "x".repeat(1_000_000)],
    ["CR-only lines", "x\r".repeat(MAX_DIFF_INPUT_LINES + 1)],
  ])("bounds %s before splitting or normalizing the original", (_, text) => {
    const diff = Object.freeze(chunk({ lines_remove: text }));
    const split = vi.spyOn(String.prototype, "split");
    const replace = vi.spyOn(String.prototype, "replace");
    const { container } = render(<DiffBlock diff={diff} uncapped />);
    const splitOriginal = split.mock.contexts.some((value) => value === text);
    const replacedOriginal = replace.mock.contexts.some(
      (value) => value === text,
    );
    split.mockRestore();
    replace.mockRestore();

    expect(isOversizedDiff(diff)).toBe(true);
    const preview = limitDiffPreview(diff);
    expect(
      fields.reduce((n, field) => n + (preview[field]?.length ?? 0), 0),
    ).toBeLessThanOrEqual(MAX_DIFF_PREVIEW_CHARS);
    expect(container.querySelectorAll("code").length).toBeLessThanOrEqual(
      MAX_DIFF_PREVIEW_LINES,
    );
    expect(
      Array.from(container.querySelectorAll("code")).reduce(
        (n, code) => n + (code.textContent?.length ?? 0),
        0,
      ),
    ).toBeLessThanOrEqual(MAX_DIFF_PREVIEW_CHARS);
    expect(splitOriginal).toBe(false);
    expect(replacedOriginal).toBe(false);
    expect(diff.lines_remove).toBe(text);
    expect(screen.getByText(/Preview limited/)).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /more diff lines|show all/i }),
    ).toBeNull();
    expect(diffStatsMeta([diff])).toBe("Large diff · line counts omitted");
    expect(getDiffStats([diff])).toEqual({
      added: 0,
      removed: 0,
      limited: true,
    });
  });

  it.each(["lines_before", "lines_after"] as const)(
    "bounds oversized %s",
    (field) => {
      const diff = chunk({ [field]: "context".repeat(MAX_DIFF_INPUT_CHARS) });
      expect(isOversizedDiff(diff)).toBe(true);
      const preview = limitDiffPreview(diff);
      expect(
        fields.reduce((n, key) => n + (preview[key]?.length ?? 0), 0),
      ).toBeLessThanOrEqual(MAX_DIFF_PREVIEW_CHARS);
      const { container } = render(<DiffBlock diff={diff} uncapped />);
      expect(container.textContent?.length).toBeLessThan(
        MAX_DIFF_PREVIEW_CHARS + 2_000,
      );
    },
  );

  it("counts aggregate fields for both safety limits", () => {
    expect(
      isOversizedDiff(
        chunk({
          lines_add: "x".repeat(60_000),
          lines_remove: "x".repeat(60_000),
        }),
      ),
    ).toBe(true);
    expect(
      isOversizedDiff(
        chunk({
          lines_before: "\n".repeat(1_001),
          lines_after: "\n".repeat(1_001),
        }),
      ),
    ).toBe(true);
    const preview = limitDiffPreview(
      chunk({
        lines_before: "x\r\n".repeat(200),
        lines_remove: "y\r".repeat(200),
        lines_add: "z\n".repeat(200),
      }),
    );
    const lineCount = fields.reduce(
      (n, field) =>
        n + ((preview[field] ?? "").match(/\r\n|\r|\n/g)?.length ?? 0),
      0,
    );
    expect(lineCount).toBe(MAX_DIFF_PREVIEW_LINES);
  });

  it("protects the uncapped EditTool path and keeps header actions", () => {
    const diff = chunk({ lines_remove: "old\n".repeat(100_000) });
    render(
      <EditTool
        toolCall={{
          id: "oversized-edit",
          index: 0,
          function: {
            name: "update_textdoc",
            arguments: '{"path":"src/demo.ts"}',
          },
        }}
        diffs={[diff]}
        isActiveTool={false}
      />,
    );
    expect(screen.getByTestId("transcript-meta")).toHaveTextContent(
      "Large diff · line counts omitted",
    );
    expect(
      screen.getByRole("button", {
        name: /Download complete original diff chunk/,
      }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /more diff lines/i }),
    ).toBeNull();
  });

  it("does not infer false deletions from truncated shared text", () => {
    const shared = "unchanged\n".repeat(MAX_DIFF_INPUT_LINES);
    const { container } = render(
      <DiffBlock
        diff={chunk({ lines_remove: shared, lines_add: shared })}
        uncapped
      />,
    );
    expect(screen.getByText("Original text excerpt")).toBeInTheDocument();
    expect(screen.getByText("Updated text excerpt")).toBeInTheDocument();
    const excerpts = container.querySelectorAll("code");
    expect(excerpts).toHaveLength(2);
    expect(excerpts[0].textContent).toBe(excerpts[1].textContent);
    expect(container.querySelector('[class*="diffLine"]')).toBeNull();
    expect(screen.queryByText(/^@@/)).toBeNull();
  });

  it("keeps both change excerpts visible when context exceeds the budget", () => {
    render(
      <DiffBlock
        diff={chunk({ lines_before: "context\n".repeat(30_000) })}
        uncapped
      />,
    );
    expect(screen.getByText("Original text excerpt")).toBeInTheDocument();
    expect(screen.getByText("Updated text excerpt")).toBeInTheDocument();
    expect(screen.getByText("old")).toBeInTheDocument();
    expect(screen.getByText("new")).toBeInTheDocument();
  });

  it("protects the actual rm FileOpTool path with truthful metadata", () => {
    const diff = chunk({
      file_action: "remove",
      lines_add: "",
      lines_remove: "deleted\n".repeat(100_000),
    });
    const { container } = render(
      <FileOpTool
        toolType="rm"
        toolCall={{
          id: "oversized-rm",
          index: 0,
          function: { name: "rm", arguments: '{"path":"src/demo.ts"}' },
        }}
        diffs={[diff]}
        isActiveTool={false}
      />,
    );
    expect(screen.getByTestId("transcript-verb")).toHaveTextContent("Delete");
    expect(screen.getByTestId("transcript-meta")).toHaveTextContent(
      "Large diff · line counts omitted",
    );
    expect(container.querySelectorAll("code").length).toBeLessThanOrEqual(
      MAX_DIFF_PREVIEW_LINES,
    );
    expect(
      screen.getByRole("button", {
        name: /Download complete original diff chunk/,
      }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /more diff lines|show all/i }),
    ).toBeNull();
  });

  it("preserves normal context, hunk headers, stats and actions", () => {
    const action = vi.fn();
    render(
      <DiffBlock
        diff={chunk({
          lines_before: "before\r\n",
          lines_after: "after\r",
          lines_remove: "old\n \n",
          lines_add: "new\n\n",
        })}
        actions={[{ label: "Apply diff", icon: "Apply", onClick: action }]}
      />,
    );
    expect(screen.getByText("@@ -10,2 +20,2 @@")).toBeInTheDocument();
    for (const text of ["before", "old", "new", "after", "+1", "−1"]) {
      expect(screen.getByText(text)).toBeInTheDocument();
    }
    fireEvent.click(screen.getByRole("button", { name: "Apply diff" }));
    expect(action).toHaveBeenCalledOnce();
    expect(
      getDiffStats([
        chunk({ lines_add: "a\n \n\t\n\r\nb", lines_remove: "x\n" }),
      ]),
    ).toEqual({ added: 2, removed: 1 });
    expect(screen.queryByText(/Preview limited/)).toBeNull();
  });

  it("keeps normal show-more behavior below the safety threshold", () => {
    render(
      <DiffBlock
        diff={chunk({
          lines_remove: "",
          lines_add: Array.from({ length: 245 }, (_, i) => `new-${i}`).join(
            "\n",
          ),
        })}
      />,
    );
    expect(screen.queryByText("new-244")).toBeNull();
    fireEvent.click(
      screen.getByRole("button", { name: "Show 5 more diff lines" }),
    );
    expect(screen.getByText("new-244")).toBeInTheDocument();
  });

  it("downloads the complete unchanged original JSON only on request and revokes URLs", async () => {
    const diff = Object.freeze(
      chunk({
        lines_remove: "old\n".repeat(30_000),
        lines_before: "before\r\n",
        lines_after: "after\r\n",
        file_name_rename: "renamed.ts",
      }),
    );
    const create = vi
      .spyOn(URL, "createObjectURL")
      .mockReturnValue("blob:diff-test");
    const revoke = vi
      .spyOn(URL, "revokeObjectURL")
      .mockImplementation(() => undefined);
    const click = vi
      .spyOn(HTMLAnchorElement.prototype, "click")
      .mockImplementation(() => undefined);
    const stringify = vi.spyOn(JSON, "stringify");
    render(<DiffBlock diff={diff} />);
    expect(create).not.toHaveBeenCalled();
    expect(stringify.mock.calls.some(([value]) => value === diff)).toBe(false);
    vi.useFakeTimers();
    fireEvent.click(
      screen.getByRole("button", {
        name: /Download complete original diff chunk/,
      }),
    );
    const blob = create.mock.calls[0][0] as Blob;
    expect(JSON.parse(await blob.text())).toEqual(diff);
    expect(blob.type).toBe("application/json");
    expect(click).toHaveBeenCalledOnce();
    expect(
      document.querySelector('a[download="original-diff-chunk.json"]'),
    ).toBeNull();
    vi.runAllTimers();
    expect(revoke).toHaveBeenCalledWith("blob:diff-test");
  });

  it("cleans download resources even if clicking fails", () => {
    vi.useFakeTimers();
    vi.spyOn(URL, "createObjectURL").mockReturnValue("blob:failed-diff");
    const revoke = vi
      .spyOn(URL, "revokeObjectURL")
      .mockImplementation(() => undefined);
    vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => {
      throw new Error("blocked");
    });
    expect(() => downloadOriginalDiff(chunk())).toThrow("blocked");
    expect(
      document.querySelector('a[download="original-diff-chunk.json"]'),
    ).toBeNull();
    vi.runAllTimers();
    expect(revoke).toHaveBeenCalledWith("blob:failed-diff");
  });
});
