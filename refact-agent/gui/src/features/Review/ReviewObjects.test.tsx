import { http, HttpResponse } from "msw";
import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { Contribution, Review } from "../../services/refact/review";
import { server } from "../../utils/mockServer";
import { render } from "../../utils/test-utils";
import { AnnotationCard, ReviewObjects, ThreadCard } from "./ReviewObjects";
import { BlockQuestion, ReviewDocumentState } from "./ReviewRichBlocks";
import { anchor, makeReview } from "./reviewFixtures";

const longBody = `${"Detailed explanation of this change. ".repeat(
  90,
)}\n\n**Final recommendation**: [Read the guide](https://example.com/guide).`;

function contribution(id: string, body = longBody): Contribution {
  return {
    id,
    thread_id: "t1",
    actor: { kind: "agent", id: "agent-1" },
    body,
    created_at_ms: 1000,
    chat_message_id: null,
    context: {
      candidate_id: "c0",
      run_id: "run-1",
      applicability: "historical",
      privacy: [],
    },
  };
}

function reviewWith(contributions: Contribution[]): Review {
  return makeReview({
    threads: [
      {
        id: "t1",
        annotation_id: null,
        anchors: [anchor("c1", "src/a.ts", "head", 3)],
        resolved: false,
        contributions,
      },
    ],
  });
}

function objects(review: Review, onFocusObject = vi.fn()) {
  return (
    <ReviewObjects
      review={review}
      candidateId="c1"
      path="src/a.ts"
      activeObjectId={null}
      onFocusObject={onFocusObject}
    />
  );
}

describe("ReviewObjects reply density", () => {
  it("shows actors in chronological order with distinct labels", () => {
    const agent = {
      ...contribution("agent", "Agent response"),
      created_at_ms: 3,
    };
    const user = {
      ...contribution("user", "User question"),
      actor: { kind: "user" as const, id: "u" },
      created_at_ms: 1,
    };
    const system = {
      ...contribution("system", "System notice"),
      actor: { kind: "system" as const, id: "s" },
      created_at_ms: 2,
    };
    const { container } = render(objects(reviewWith([agent, user, system])));
    const entries = [...container.querySelectorAll("[data-actor]")];
    expect(entries.map((entry) => entry.getAttribute("data-actor"))).toEqual([
      "user",
      "system",
      "agent",
    ]);
    expect(entries[0]).toHaveTextContent("You");
    expect(entries[1]).toHaveTextContent("System");
    expect(entries[2]).toHaveTextContent("Agent");
  });
  it("keeps an unplaced current BASE thread visible in since-last fallback", () => {
    const review = reviewWith([contribution("base", "Base discussion")]);
    review.threads[0].anchors = [anchor("c1", "src/a.ts", "base", 1)];
    const { rerender } = render(
      <ReviewObjects
        review={review}
        candidateId="c1"
        path="src/a.ts"
        activeObjectId={null}
        onFocusObject={() => undefined}
        fallbackOnly
        placedIds={new Set()}
      />,
    );
    expect(screen.getByText("Base discussion")).toBeInTheDocument();
    rerender(
      <ReviewObjects
        review={review}
        candidateId="c1"
        path="src/a.ts"
        activeObjectId={null}
        onFocusObject={() => undefined}
        fallbackOnly
        placedIds={new Set(["thread:t1"])}
      />,
    );
    expect(screen.queryByText("Base discussion")).not.toBeInTheDocument();
  });
  it("expands and collapses one unchanged Markdown body without review writes or focus changes", async () => {
    const writes = vi.fn();
    server.use(
      http.all("*/v1/reviews/*", ({ request }) => {
        if (request.method !== "GET") writes();
        return HttpResponse.json([]);
      }),
    );
    const review = reviewWith([contribution("reply-1")]);
    const original = JSON.stringify(review);
    const onFocusObject = vi.fn();
    const { user } = render(objects(review, onFocusObject));
    const toggle = screen.getByRole("button", { name: "Show full reply" });
    const body = document.getElementById(
      toggle.getAttribute("aria-controls") ?? "",
    );
    if (!body) throw new Error("Reply body is missing");
    expect(body).toHaveAttribute("data-clamped", "true");
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(screen.getAllByText("Final recommendation")).toHaveLength(1);
    const markup = body.innerHTML;
    expect(body).toHaveTextContent("Detailed explanation of this change.");

    await user.click(toggle);
    expect(toggle).toHaveAccessibleName("Show less");
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(toggle).toHaveFocus();
    expect(body).toHaveAttribute("data-clamped", "false");
    expect(body.innerHTML).toBe(markup);
    expect(
      within(body).getByRole("link", { name: "Read the guide" }),
    ).toHaveAttribute("href", "https://example.com/guide");
    await user.click(toggle);
    expect(toggle).toHaveAccessibleName("Show full reply");
    expect(body).toHaveAttribute("data-clamped", "true");
    expect(body.innerHTML).toBe(markup);
    expect(JSON.stringify(review)).toBe(original);
    expect(screen.getByText("Agent").parentElement).toHaveTextContent(
      /on an earlier candidate.*run run-1.*historical/,
    );
    expect(screen.queryByText("Resolved")).not.toBeInTheDocument();
    expect(onFocusObject).not.toHaveBeenCalled();
    expect(writes).not.toHaveBeenCalled();
  });

  it("keeps expansion isolated by contribution id across asynchronous review updates and reordering", async () => {
    const first = contribution("first");
    const second = contribution("second");
    const { user, rerender } = render(objects(reviewWith([first, second])));
    const toggles = screen.getAllByRole("button", { name: "Show full reply" });
    const firstBodyId = toggles[0].getAttribute("aria-controls");
    const secondBodyId = toggles[1].getAttribute("aria-controls");
    await user.click(toggles[0]);
    expect(toggles[1]).toHaveAttribute("aria-expanded", "false");

    rerender(objects({ ...reviewWith([second, { ...first }]), revision: 4 }));
    expect(screen.getByRole("button", { name: "Show less" })).toHaveAttribute(
      "aria-controls",
      firstBodyId,
    );
    expect(
      screen.getByRole("button", { name: "Show full reply" }),
    ).toHaveAttribute("aria-controls", secondBodyId);
  });

  it("leaves small user comments open and preserves their pending delivery status", async () => {
    server.use(
      http.get("*/v1/reviews/r1/delivery-status", () =>
        HttpResponse.json([
          {
            id: "d1",
            review_id: "r1",
            contribution_id: "user-1",
            owner_chat_id: "chat-1",
            delivered: false,
          },
        ]),
      ),
    );
    render(
      objects(
        reviewWith([
          {
            ...contribution("user-1", "Please check this edge case."),
            actor: { kind: "user", id: "u1" },
          },
        ]),
      ),
    );
    expect(
      screen.getByText("Please check this edge case."),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Show full reply" }),
    ).not.toBeInTheDocument();
    expect(
      await screen.findByText(/Saved to review · awaiting chat delivery/),
    ).toBeInTheDocument();
  });

  it("collapses line-heavy replies even when their character count is small", () => {
    render(
      objects(
        reviewWith([
          contribution("lines", "one\ntwo\nthree\nfour\nfive\nsix\nseven"),
        ]),
      ),
    );
    expect(
      screen.getByRole("button", { name: "Show full reply" }),
    ).toHaveAttribute("aria-expanded", "false");
  });
});

describe("shared review composer ownership", () => {
  it("reopens a displaced thread with the same mounted textarea and draft", async () => {
    const review = reviewWith([contribution("reply", "Discussion")]);
    const tree = (revision: number) => (
      <ReviewDocumentState>
        <ThreadCard
          review={{ ...review, revision }}
          thread={review.threads[0]}
          active={false}
          pending={new Set()}
          onFocus={vi.fn()}
        />
        <BlockQuestion
          review={review}
          target={{ candidate_id: "c1", step_id: "alpha", version: 1 }}
        />
      </ReviewDocumentState>
    );
    const { user, rerender } = render(tree(1));
    await user.click(screen.getByRole("button", { name: "Reply" }));
    const reply = screen.getByRole("textbox", { name: "Reply" });
    await user.type(reply, "Retained reply");
    await user.click(
      screen.getByRole("button", { name: "Ask about this block" }),
    );
    expect(screen.getAllByRole("textbox")).toHaveLength(1);
    rerender(tree(2));
    expect(
      screen.getByRole("textbox", { name: "Question about this block" }),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Reply" }));
    expect(screen.getByRole("textbox", { name: "Reply" })).toBe(reply);
    expect(reply).toHaveValue("Retained reply");
  });

  it("allows only one mounted representation of a thread to own its shared draft", async () => {
    const review = reviewWith([contribution("reply", "Discussion")]);
    const { user } = render(
      <ReviewDocumentState>
        {["first", "second"].map((id) => (
          <section key={id} aria-label={id}>
            <ThreadCard
              review={review}
              thread={review.threads[0]}
              active={false}
              pending={new Set()}
              onFocus={vi.fn()}
            />
          </section>
        ))}
      </ReviewDocumentState>,
    );
    const first = within(screen.getByRole("region", { name: "first" }));
    const second = within(screen.getByRole("region", { name: "second" }));
    await user.click(first.getByRole("button", { name: "Reply" }));
    await user.type(first.getByRole("textbox"), "One semantic draft");
    await user.click(second.getByRole("button", { name: "Reply" }));
    expect(screen.getAllByRole("textbox")).toHaveLength(1);
    expect(second.getByRole("textbox")).toHaveValue("One semantic draft");
    await user.click(first.getByRole("button", { name: "Reply" }));
    expect(screen.getAllByRole("textbox")).toHaveLength(1);
  });

  it("keeps annotation drafts across displacement and cancel, preserving the request target", async () => {
    const review = makeReview();
    const annotation = {
      id: "a1",
      run_id: null,
      kind: "finding" as const,
      title: "Exact finding",
      body: "Finding body",
      anchors: [anchor("c1", "src/a.ts", "head", 3)],
      severity: "medium" as const,
      assessment: "supported" as const,
      disposition: "open" as const,
      applicability: "current" as const,
      confidence: "high" as const,
      evidence_ids: [],
    };
    const requests: unknown[] = [];
    server.use(
      http.post("*/v1/reviews/r1/comments", async ({ request }) => {
        requests.push(await request.json());
        return HttpResponse.json(makeReview());
      }),
    );
    const { user } = render(
      <ReviewDocumentState>
        <AnnotationCard
          review={review}
          annotation={annotation}
          active={false}
          onFocus={vi.fn()}
        />
        <BlockQuestion
          review={review}
          target={{ candidate_id: "c1", step_id: "alpha", version: 1 }}
        />
      </ReviewDocumentState>,
    );
    screen.getByRole("button", { name: "Discuss" }).focus();
    await user.keyboard("{Enter}");
    const textarea = screen.getByRole("textbox", {
      name: "Discuss “Exact finding”",
    });
    expect(textarea).toHaveFocus();
    await user.type(textarea, "Annotation draft");
    await user.keyboard("{Escape}");
    expect(screen.getByRole("button", { name: "Discuss" })).toHaveFocus();
    await user.keyboard(" ");
    expect(screen.getByRole("textbox")).toBe(textarea);
    expect(textarea).toHaveFocus();
    expect(textarea).toHaveValue("Annotation draft");
    await user.click(
      screen.getByRole("button", { name: "Ask about this block" }),
    );
    expect(screen.getAllByRole("textbox")).toHaveLength(1);
    await user.click(screen.getByRole("button", { name: "Discuss" }));
    expect(screen.getByRole("textbox")).toBe(textarea);
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.getByRole("button", { name: "Discuss" })).toHaveFocus();
    await user.click(screen.getByRole("button", { name: "Discuss" }));
    expect(textarea).not.toHaveFocus();
    expect(textarea).toHaveValue("Annotation draft");
    const form = textarea.closest("form");
    if (!form) throw new Error("Missing annotation form");
    textarea.focus();
    await user.keyboard("{Control>}{Enter}{/Control}");
    fireEvent.submit(form);
    await waitFor(() => expect(requests).toHaveLength(1));
    expect(requests[0]).toMatchObject({
      annotation_id: "a1",
      anchor: annotation.anchors[0],
      body: "Annotation draft",
    });
  });
});
