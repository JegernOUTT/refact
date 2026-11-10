import { describe, expect, it } from "vitest";
import {
  filePreview,
  previewReference,
  ReviewHtmlPage,
} from "./ReviewHtmlPage";
import { makeReview } from "./reviewFixtures";
import type { RichBlock, ReviewFileView } from "../../services/refact/review";

const diff: RichBlock = {
  id: "diff",
  revision: 1,
  content: { kind: "diff", candidate_id: "c1", path: "src/a.ts" },
};
const file: ReviewFileView = {
  path: "src/a.ts",
  base: { kind: "text", text: "old" },
  head: { kind: "text", text: "new" },
  diff: "",
};

describe("HTML page pinned previews", () => {
  it("retains final lines without trailing newlines, and identifies newline differences", () => {
    const preview = filePreview(diff, file);
    expect(preview).toContain("-old\n");
    expect(preview).toContain("+new\n");
    expect(preview).toContain("No newline at end of file");
  });
  it("shows actual change hunks after a long unchanged prefix", () => {
    const prefix = Array.from({ length: 300 }, (_, i) => `same ${i}\n`).join(
      "",
    );
    const preview = filePreview(diff, {
      ...file,
      base: { kind: "text", text: prefix + "old\n" },
      head: { kind: "text", text: prefix + "new\n" },
    });
    expect(preview).toContain("-old");
    expect(preview).toContain("+new");
    expect(preview).not.toContain("same 0\n");
  });
  it("rejects arbitrary paths and cross-candidate references", () => {
    const review = makeReview();
    expect(previewReference(review, "c2", diff)).toBeNull();
    expect(
      previewReference(review, "c1", {
        ...diff,
        content: { kind: "diff", candidate_id: "c1", path: "/etc/passwd" },
      }),
    ).toBeNull();
  });
  it("preserves code characters verbatim rather than parsing source as markup", () => {
    const anchor = {
      position: {
        candidate_id: "c1",
        path: "src/a.ts",
        side: "head" as const,
        start_line: 1,
        end_line: 1,
        content_hash: "",
        context_before: [],
        context_after: [],
      },
      status: "current" as const,
      stalled_reason: null,
      history: [],
    };
    const text = '\tconst html = "</script><img src=x onerror=alert(1)>";\r';
    expect(
      filePreview(
        { id: "code", revision: 1, content: { kind: "code", anchor } },
        { ...file, head: { kind: "text", text } },
      ),
    ).toBe(text);
  });
});

it("focuses an explicitly requested HTML block discussion", async () => {
  const { render } = await import("../../utils/test-utils");
  const { screen, waitFor } = await import("@testing-library/react");
  const { RichDiscussionContext } = await import("./ReviewRichBlocks");
  const target = {
    candidate_id: "c1",
    step_id: "html",
    version: 1,
    block_id: "evidence",
    block_revision: 1,
  };
  const review = makeReview({
    threads: [
      {
        id: "exact-thread",
        target,
        anchors: [],
        annotation_id: null,
        resolved: false,
        contributions: [],
      },
    ],
  });
  const version = {
    version: 1,
    title: "HTML",
    body: "",
    html: "<h1>HTML</h1>",
    anchors: [],
    blocks: [
      {
        id: "evidence",
        revision: 1,
        content: { kind: "evidence" as const, evidence_ids: [] },
      },
    ],
  };
  render(
    <RichDiscussionContext.Provider value={(id) => <p>Discussion {id}</p>}>
      <ReviewHtmlPage
        review={review}
        version={version}
        stepId="html"
        target={target}
        revealTarget={{ target, token: 4, threadId: "exact-thread" }}
      />
    </RichDiscussionContext.Provider>,
  );
  await waitFor(() =>
    expect(
      screen
        .getByText("Discussion exact-thread")
        .closest("[data-document-thread-id]"),
    ).toHaveFocus(),
  );
});
