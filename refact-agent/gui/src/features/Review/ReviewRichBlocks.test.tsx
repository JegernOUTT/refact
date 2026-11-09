import { EditorView } from "@codemirror/view";
import { fireEvent, screen, waitFor } from "@testing-library/react";
import { act } from "react-dom/test-utils";
import { http, HttpResponse } from "msw";
import { describe, expect, it, vi } from "vitest";
import { render } from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import {
  BlockQuestion,
  ReviewRichBlocks,
  ReviewDocumentState,
  RichNavigationContext,
  RichFileNavigationContext,
  RichDiscussionContext,
} from "./ReviewRichBlocks";
import { anchor, bytes, makeReview } from "./reviewFixtures";
import { ThreadCard } from "./ReviewObjects";
import { ReviewReadOnlyContext } from "./readOnly";
import { ArtifactBlock } from "../../components/Markdown/ArtifactBlock";
import { ArtifactPolicyProvider } from "../../components/Markdown/ArtifactPolicyContext";

describe("ReviewRichBlocks", () => {
  it("never mounts a replacement reduced-motion artifact using the prior artifact's live consent", async () => {
    const original = window.matchMedia;
    window.matchMedia = vi.fn().mockReturnValue({
      matches: true,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    });
    try {
      const view = (
        code: string,
        bindingKey: string,
        staticFallback?: string,
      ) => (
        <ArtifactPolicyProvider value={{ mode: "review-seamless", bindingKey }}>
          <ArtifactBlock code={code} staticFallback={staticFallback} />
        </ArtifactPolicyProvider>
      );
      const { user, rerender, container } = render(
        view(
          "<script>window.firstStarted = true</script>",
          "first",
          "First sequence",
        ),
      );
      expect(container.querySelector("iframe")).toBeNull();
      await user.click(
        screen.getByRole("button", { name: "Show live explanation" }),
      );
      expect(container.querySelector("iframe")).not.toBeNull();
      const mountedFrames: Element[] = [];
      const observer = new MutationObserver((records) =>
        records.forEach((record) =>
          record.addedNodes.forEach((node) => {
            if (node instanceof Element) {
              if (node.matches("iframe")) mountedFrames.push(node);
              mountedFrames.push(...node.querySelectorAll("iframe"));
            }
          }),
        ),
      );
      observer.observe(container, { childList: true, subtree: true });
      rerender(view("<script>second()</script>", "second", "Second sequence"));
      await Promise.resolve();
      expect(container.querySelector("iframe")).toBeNull();
      expect(mountedFrames).toHaveLength(0);
      rerender(view("<script>legacy()</script>", "legacy", " "));
      await Promise.resolve();
      expect(container.querySelector("iframe")).toBeNull();
      expect(mountedFrames).toHaveLength(0);
      expect(
        screen.getByText(/No static explanation was provided/),
      ).toBeVisible();
      expect(screen.getByText("<script>legacy()</script>")).toBeVisible();
      observer.disconnect();
    } finally {
      window.matchMedia = original;
    }
  });

  it("offers a keyboard static alternative outside the native iframe and restarts only on explicit live switch", async () => {
    const blocks = [
      {
        id: "dynamic",
        revision: 1,
        content: {
          kind: "html" as const,
          html: "<p>Interactive sequence</p>",
          static_fallback:
            "Caller → split → trim → convert → result. <script>inert</script>",
        },
      },
    ];
    const { user, rerender } = render(
      <ReviewRichBlocks
        review={makeReview()}
        body=""
        bindingKey="r1:c1"
        blocks={blocks}
      />,
    );
    await user.click(
      screen.getByRole("button", { name: "Load interactive block" }),
    );
    const frame = screen.getByTitle("Tour visual");
    rerender(
      <ReviewRichBlocks
        review={makeReview({ revision: 5 })}
        body="Updated explanation"
        bindingKey="r1:c1"
        blocks={blocks}
      />,
    );
    expect(screen.getByTitle("Tour visual")).toBe(frame);
    screen.getByRole("button", { name: "Show static explanation" }).focus();
    await user.keyboard("{Enter}");
    expect(screen.queryByTitle("Tour visual")).not.toBeInTheDocument();
    const fallback = screen.getByRole("region", { name: "Static explanation" });
    expect(fallback).toHaveTextContent(blocks[0].content.static_fallback);
    expect(fallback.querySelector("script")).toBeNull();
    expect(fallback).toHaveTextContent("Switching to live restarts it");
    await user.keyboard("{Enter}");
    expect(screen.getByTitle("Tour visual")).not.toBe(frame);
  });

  it("refuses empty, read-only and in-flight direct question submissions", async () => {
    let release!: () => void;
    const pending = new Promise<void>((resolve) => {
      release = resolve;
    });
    const requests = vi.fn();
    server.use(
      http.post("*/v1/reviews/r1/comments", async () => {
        requests();
        await pending;
        return HttpResponse.json(makeReview());
      }),
    );
    const question = (readOnly: boolean) => (
      <ReviewReadOnlyContext.Provider value={readOnly}>
        <BlockQuestion
          review={makeReview()}
          target={{ candidate_id: "c1", step_id: "alpha", version: 1 }}
        />
      </ReviewReadOnlyContext.Provider>
    );
    const { user, rerender } = render(question(false));
    await user.click(
      screen.getByRole("button", { name: "Ask about this block" }),
    );
    const textarea = screen.getByRole("textbox", {
      name: "Question about this block",
    });
    const form = textarea.closest("form");
    if (!form) throw new Error("Question textarea has no form");
    fireEvent.submit(form);
    expect(requests).not.toHaveBeenCalled();
    await user.type(textarea, "A retained question");
    rerender(question(true));
    fireEvent.submit(form);
    expect(requests).not.toHaveBeenCalled();
    rerender(question(false));
    act(() => {
      form.dispatchEvent(
        new Event("submit", { bubbles: true, cancelable: true }),
      );
      form.dispatchEvent(
        new Event("submit", { bubbles: true, cancelable: true }),
      );
    });
    await waitFor(() => expect(requests).toHaveBeenCalledTimes(1));
    fireEvent.submit(form);
    expect(requests).toHaveBeenCalledTimes(1);
    release();
    await waitFor(() => expect(textarea).toHaveValue(""));
  });
  it("keeps a question draft mounted across accessible disclosure toggles", async () => {
    const { user } = render(
      <BlockQuestion
        review={makeReview()}
        target={{ candidate_id: "c1", step_id: "alpha", version: 1 }}
      />,
    );
    const toggle = screen.getByRole("button", { name: "Ask about this block" });
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(toggle).toHaveAttribute("aria-controls");
    await user.click(toggle);
    const textarea = screen.getByRole("textbox", {
      name: "Question about this block",
    });
    await user.type(textarea, "Does this preserve state?");
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    await user.click(toggle);
    expect(
      screen.getByRole("textbox", { name: "Question about this block" }),
    ).toBe(textarea);
    expect(textarea).toHaveValue("Does this preserve state?");
    expect(toggle).toHaveFocus();
  });

  it("keeps one active question and restores each draft and identical uncertain retry", async () => {
    const requests: unknown[] = [];
    server.use(
      http.post("*/v1/reviews/r1/comments", async ({ request }) => {
        requests.push(await request.json());
        return requests.length === 1
          ? HttpResponse.json({ detail: "Unavailable" }, { status: 503 })
          : HttpResponse.json(makeReview());
      }),
    );
    const { user } = render(
      <ReviewDocumentState>
        {["first", "second"].map((id) => (
          <BlockQuestion
            key={id}
            review={makeReview()}
            target={{
              candidate_id: "c1",
              step_id: id,
              version: 1,
              block_id: "b",
              block_revision: 2,
            }}
          />
        ))}
      </ReviewDocumentState>,
    );
    const toggles = screen.getAllByRole("button", {
      name: "Ask about this block",
    });
    toggles[0].focus();
    await user.keyboard("{Enter}");
    const first = screen.getByRole("textbox", {
      name: "Question about this block",
    });
    expect(toggles[0]).toHaveFocus();
    await user.type(first, "First retained draft");
    await user.click(toggles[1]);
    expect(
      screen.getAllByRole("textbox", { name: "Question about this block" }),
    ).toHaveLength(1);
    await user.type(
      screen.getByRole("textbox", { name: "Question about this block" }),
      "Second draft",
    );
    await user.click(toggles[0]);
    expect(
      screen.getByRole("textbox", { name: "Question about this block" }),
    ).toBe(first);
    expect(first).toHaveValue("First retained draft");
    await user.click(screen.getByRole("button", { name: "Ask" }));
    await screen.findByRole("alert");
    await user.click(toggles[1]);
    expect(
      screen.getByRole("textbox", { name: "Question about this block" }),
    ).toHaveValue("Second draft");
    await user.click(toggles[0]);
    await user.click(screen.getByRole("button", { name: "Retry question" }));
    await waitFor(() => expect(requests).toHaveLength(2));
    expect(requests[1]).toEqual(requests[0]);
  });

  it("separates duplicate mounted block owners while sharing only exact target drafts", async () => {
    const target = {
      candidate_id: "c1",
      step_id: "alpha",
      version: 1,
      block_id: "b",
      block_revision: 2,
    };
    const { user } = render(
      <ReviewDocumentState>
        {[target, { ...target }, { ...target, block_revision: 3 }].map(
          (value, index) => (
            <BlockQuestion key={index} review={makeReview()} target={value} />
          ),
        )}
      </ReviewDocumentState>,
    );
    const triggers = screen.getAllByRole("button", {
      name: "Ask about this block",
    });
    await user.click(triggers[0]);
    await user.type(screen.getByRole("textbox"), "Exact revision draft");
    await user.click(triggers[1]);
    expect(screen.getAllByRole("textbox")).toHaveLength(1);
    expect(screen.getByRole("textbox")).toHaveValue("Exact revision draft");
    expect(triggers[0]).toHaveAttribute("aria-expanded", "false");
    await user.click(triggers[2]);
    expect(screen.getByRole("textbox")).toHaveValue("");
    await user.click(triggers[0]);
    expect(screen.getByRole("textbox")).toHaveValue("Exact revision draft");
  });

  it("keeps distinct evidence on the same path and exposes precise versus stalled navigation", async () => {
    server.use(
      http.get("*/v1/reviews/r1/files", () =>
        HttpResponse.json({
          path: "src/a.ts",
          base: bytes("one\ntwo\nthree\nfour\nfive\n"),
          head: bytes("one\ntwo\nthree\nfour\nfive\n"),
          diff: "",
        }),
      ),
    );
    const exact = anchor("c1", "src/a.ts", "head", 2);
    exact.position.end_line = 4;
    const stalled = {
      ...exact,
      status: "stalled" as const,
      stalled_reason: "Content changed",
    };
    const evidence = [exact, stalled, null].map((a, index) => ({
      id: `e${index}`,
      candidate_id: "c1",
      material_id: `m${index}`,
      run_id: null,
      source: { kind: "source" as const, path: "src/a.ts", anchor: a },
      summary: `Reason ${index}`,
      applicability: "current" as const,
      created_at_ms: 1,
    }));
    const navigate = vi.fn();
    const file = vi.fn();
    const { user } = render(
      <RichNavigationContext.Provider value={navigate}>
        <RichFileNavigationContext.Provider value={file}>
          <ReviewRichBlocks
            review={makeReview({ evidence })}
            body=""
            bindingKey="evidence"
            blocks={[
              {
                id: "e",
                revision: 1,
                content: {
                  kind: "sources",
                  relation: "supports",
                  evidence_ids: ["e0", "e1", "e2", "e0"],
                },
              },
            ]}
          />
        </RichFileNavigationContext.Provider>
      </RichNavigationContext.Provider>,
    );
    await user.click(
      screen.getByRole("button", { name: "Load interactive block" }),
    );
    expect(screen.getAllByText(/Reason/)).toHaveLength(3);
    expect(screen.getByText("Stalled · file only")).toBeInTheDocument();
    expect(screen.getByText("File only · no exact range")).toBeInTheDocument();
    const paths = screen.getAllByRole("button", { name: /src\/a.ts/ });
    await user.click(paths[0]);
    expect(navigate).toHaveBeenCalledWith(exact);
    await user.click(paths[1]);
    expect(file).toHaveBeenCalledWith("c1", "src/a.ts");
    expect(navigate).toHaveBeenCalledTimes(1);
    expect(
      screen.getByRole("button", { name: "Inspect exact range" }),
    ).toBeInTheDocument();
    await user.click(
      screen.getByRole("button", { name: "Inspect exact range" }),
    );
    const context = screen.getByRole("region", {
      name: "Exact source context",
    });
    await waitFor(() =>
      expect(context.querySelector(".cm-editor")).not.toBeNull(),
    );
    const editor = context.querySelector<HTMLElement>(".cm-editor");
    if (!editor) throw new Error("Exact context editor did not mount");
    const view = EditorView.findFromDOM(editor);
    expect(view?.state.doc.toString()).toContain("five");
    expect(context).toHaveTextContent("head L2–4");
    await waitFor(() =>
      expect(context.querySelector(".rf-review-hl-tour")).not.toBeNull(),
    );
  });

  it("preserves legacy body fallback", () => {
    render(
      <ReviewRichBlocks
        review={makeReview()}
        body="Legacy explanation"
        bindingKey="r1:c1"
      />,
    );
    expect(screen.getByText("Legacy explanation")).toBeInTheDocument();
  });
  it("preserves distinct explanatory prose alongside eager structured markdown", () => {
    render(
      <ReviewRichBlocks
        review={makeReview()}
        body="Fallback"
        bindingKey="r1:c1"
        blocks={[
          {
            id: "b",
            revision: 1,
            content: { kind: "markdown", body: "Rich explanation" },
          },
        ]}
      />,
    );
    expect(screen.getByText("Rich explanation")).toBeInTheDocument();
    expect(screen.getByText("Fallback")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Load interactive block" }),
    ).not.toBeInTheDocument();
  });
  it("shows exactly matching Markdown immediately without duplicating the body", () => {
    render(
      <ReviewRichBlocks
        review={makeReview()}
        body="Exact explanation"
        bindingKey="r1:dedup"
        blocks={[
          {
            id: "same",
            revision: 1,
            content: { kind: "markdown", body: "Exact explanation" },
          },
        ]}
      />,
    );
    expect(screen.getAllByText("Exact explanation")).toHaveLength(1);
  });
  it("preserves whitespace-significant Markdown instead of deduplicating by trimmed text", () => {
    const { container } = render(
      <ReviewRichBlocks
        review={makeReview()}
        body="    critical"
        bindingKey="r1:whitespace"
        blocks={[
          {
            id: "paragraph",
            revision: 1,
            content: { kind: "markdown", body: "critical" },
          },
        ]}
      />,
    );
    expect(container.querySelector("pre code")).toHaveTextContent("critical");
    expect(
      container.querySelector('[data-block-id="paragraph"] p'),
    ).toHaveTextContent("critical");
  });
  it("preserves prose when a Markdown block contains only part of it", () => {
    render(
      <ReviewRichBlocks
        review={makeReview()}
        body="Full explanation with an additional caveat."
        bindingKey="r1:partial"
        blocks={[
          {
            id: "partial",
            revision: 1,
            content: { kind: "markdown", body: "Full explanation" },
          },
        ]}
      />,
    );
    expect(
      screen.getByText("Full explanation with an additional caveat."),
    ).toBeInTheDocument();
    expect(screen.getByText("Full explanation")).toBeInTheDocument();
  });
  it("shows explanatory prose with native code and source blocks before they mount", () => {
    render(
      <ReviewRichBlocks
        review={makeReview()}
        body="Recovery contract remains readable"
        bindingKey="r1:code"
        blocks={[
          {
            id: "code",
            revision: 1,
            content: {
              kind: "code",
              anchor: anchor("c1", "src/a.ts", "head", 1),
            },
          },
          {
            id: "sources",
            revision: 1,
            content: {
              kind: "sources",
              evidence_ids: [],
              relation: "supports",
            },
          },
        ]}
      />,
    );
    expect(
      screen.getByText("Recovery contract remains readable"),
    ).toBeInTheDocument();
    expect(
      screen.getAllByRole("button", { name: "Load interactive block" }),
    ).toHaveLength(2);
  });
  it.each(["owned", "range", "other-revision", "standalone"] as const)(
    "renders one native discussion for %s thread ownership",
    async (ownership) => {
      server.use(
        http.get("*/v1/reviews/r1/files", () =>
          HttpResponse.json({
            path: "src/a.ts",
            base: bytes("unchanged\nbefore\n"),
            head: bytes("unchanged\nafter\n"),
            diff: "",
          }),
        ),
      );
      const a = anchor("c1", "src/a.ts", "head", 2);
      const target = { candidate_id: "c1", step_id: "alpha", version: 1 };
      const thread = {
        id: "reader-thread",
        anchors: [a],
        annotation_id: null,
        resolved: false,
        contributions: [],
        ...(ownership === "range"
          ? {}
          : {
              target: {
                ...target,
                block_id: "code",
                block_revision: ownership === "other-revision" ? 2 : 1,
              },
            }),
      };
      const review = makeReview({ threads: [thread] });
      const { user, container } = render(
        <RichDiscussionContext.Provider
          value={() => (
            <ThreadCard
              review={review}
              thread={thread}
              active={false}
              pending={new Set()}
              onFocus={vi.fn()}
            />
          )}
        >
          <ReviewRichBlocks
            review={review}
            body=""
            bindingKey="r1:alpha"
            target={ownership === "standalone" ? undefined : target}
            blocks={[
              { id: "code", revision: 1, content: { kind: "code", anchor: a } },
            ]}
          />
        </RichDiscussionContext.Provider>,
      );
      if (ownership === "owned")
        expect(
          screen.getByRole("article", { name: /Comment thread/ }),
        ).toBeVisible();
      for (const button of screen.getAllByRole("button", {
        name: "Load interactive block",
      }))
        await user.click(button);
      await screen.findByRole("textbox", { name: /^Head/ });
      await waitFor(() => {
        expect(container.querySelectorAll(".cm-editor")).toHaveLength(1);
        expect(
          screen.getAllByRole("article", { name: /Comment thread/ }),
        ).toHaveLength(1);
      });
      const canonical = container.querySelector("[data-document-thread-id]");
      if (ownership === "owned") {
        expect(canonical).toBeVisible();
        expect(
          container.querySelector(".rf-review-inline-discussion"),
        ).toBeNull();
      } else {
        expect(canonical).toBeNull();
        expect(
          container.querySelector(".rf-review-inline-discussion"),
        ).toBeVisible();
      }
    },
  );

  it("mounts real native editors for code and diff including unchanged context", async () => {
    server.use(
      http.get("*/v1/reviews/r1/files", () =>
        HttpResponse.json({
          path: "src/a.ts",
          base: bytes("unchanged\nbefore\n"),
          head: bytes("unchanged\nafter\n"),
          diff: "",
        }),
      ),
    );
    const review = makeReview();
    const a = anchor("c1", "src/a.ts", "head", 2);
    const { user, container } = render(
      <ReviewRichBlocks
        review={review}
        body=""
        bindingKey="r1:c1"
        blocks={[
          { id: "code", revision: 1, content: { kind: "code", anchor: a } },
          {
            id: "diff",
            revision: 1,
            content: {
              kind: "diff",
              candidate_id: "c1",
              path: "src/a.ts",
              anchors: [a],
            },
          },
        ]}
      />,
    );
    for (const button of screen.getAllByRole("button", {
      name: "Load interactive block",
    }))
      await user.click(button);
    expect(
      await screen.findAllByRole("textbox", { name: /^Head/ }),
    ).not.toHaveLength(0);
    expect(
      container.querySelectorAll(".cm-editor").length,
    ).toBeGreaterThanOrEqual(2);
    expect(container.textContent).toContain("unchanged");
    const editors = Array.from(container.querySelectorAll(".cm-editor"));
    const metadata = screen.getAllByRole("button", {
      name: "Reference metadata",
    })[0];
    expect(metadata).toHaveAttribute("aria-expanded", "false");
    await user.click(metadata);
    expect(metadata).toHaveAttribute("aria-expanded", "true");
    expect(metadata).toHaveAttribute("aria-controls");
    expect(
      screen.getAllByText(review.candidates[0].content_digest).length,
    ).toBeGreaterThan(0);
    await user.click(metadata);
    expect(container.querySelectorAll(".cm-editor")).toHaveLength(
      editors.length,
    );
    editors.forEach((editor, index) => {
      expect(container.querySelectorAll(".cm-editor")[index]).toBe(editor);
    });
  });
  it("separates consulted and supporting sources from executed evidence and unknown IDs", async () => {
    const review = makeReview({
      evidence: [
        {
          id: "src",
          candidate_id: "c1",
          material_id: null,
          run_id: null,
          source: { kind: "external", url: "https://example.com/reference" },
          summary: "Reference source",
          applicability: "current",
          created_at_ms: 1,
        },
        {
          id: "exec",
          candidate_id: "c1",
          material_id: null,
          run_id: null,
          source: {
            kind: "executed",
            command: ["npm", "test"],
            workdir: "/repo",
            exit_code: 0,
            stdout: null,
            stderr: null,
          },
          summary: "Test run",
          applicability: "current",
          created_at_ms: 1,
        },
      ],
    });
    const { user } = render(
      <ReviewRichBlocks
        review={review}
        body=""
        bindingKey="r1:c1"
        blocks={[
          {
            id: "consulted",
            revision: 1,
            content: {
              kind: "sources",
              relation: "consulted",
              evidence_ids: ["src", "exec", "missing"],
            },
          },
          {
            id: "support",
            revision: 1,
            content: {
              kind: "sources",
              relation: "supports",
              evidence_ids: ["src"],
            },
          },
          {
            id: "evidence",
            revision: 1,
            content: { kind: "evidence", evidence_ids: ["exec", "missing"] },
          },
        ]}
      />,
    );
    for (const button of screen.getAllByRole("button", {
      name: "Load interactive block",
    }))
      await user.click(button);
    expect(
      screen.getByRole("region", { name: "consulted sources" }),
    ).toHaveTextContent("Reference source");
    expect(
      screen.getByRole("region", { name: "consulted sources" }),
    ).not.toHaveTextContent("npm test");
    expect(
      screen.getByRole("region", { name: "supports sources" }),
    ).toHaveTextContent("Reference source");
    expect(screen.getByRole("region", { name: "Evidence" })).toHaveTextContent(
      "Executed: npm test",
    );
    expect(screen.queryByText("missing")).not.toBeInTheDocument();
  });
  it("preserves the sandboxed HTML iframe on unrelated review updates", async () => {
    const review = makeReview();
    const blocks = [
      {
        id: "html",
        revision: 1,
        content: { kind: "html" as const, html: "<p>Visual</p>", anchors: [] },
      },
    ];
    const { user, rerender } = render(
      <ReviewRichBlocks
        review={review}
        body=""
        bindingKey="r1:c1"
        blocks={blocks}
      />,
    );
    await user.click(
      screen.getByRole("button", { name: "Load interactive block" }),
    );
    const frame = await screen.findByTitle("Tour visual");
    expect(frame).toHaveAttribute("sandbox");
    rerender(
      <ReviewRichBlocks
        review={{ ...review, revision: 4 }}
        body="Explanation added beside the interactive visual"
        bindingKey="r1:c1"
        blocks={blocks}
      />,
    );
    expect(
      screen.getByText("Explanation added beside the interactive visual"),
    ).toBeInTheDocument();
    expect(screen.getByTitle("Tour visual")).toBe(frame);
  });
});
