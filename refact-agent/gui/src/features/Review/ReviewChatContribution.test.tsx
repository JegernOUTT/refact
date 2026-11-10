import { screen } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { describe, expect, it, vi } from "vitest";
import { server } from "../../utils/mockServer";
import { render } from "../../utils/test-utils";
import { makeReview } from "./reviewFixtures";
import {
  isRichReviewContribution,
  ReviewChatContribution,
} from "./ReviewChatContribution";

const blocks = [
  {
    id: "b1",
    revision: 1,
    content: { kind: "markdown" as const, body: "Canonical rich reply" },
  },
];
const extra = {
  review: { id: "r1", thread_id: "t1", contribution_id: "c1", blocks },
};
const review = makeReview({
  threads: [
    {
      id: "t1",
      anchors: [],
      annotation_id: null,
      resolved: false,
      contributions: [
        {
          id: "c1",
          thread_id: "t1",
          actor: { kind: "agent", id: "agent" },
          body: "canonical body",
          blocks,
          created_at_ms: 1,
          chat_message_id: null,
          context: {
            candidate_id: "older",
            run_id: null,
            applicability: "historical",
            privacy: [],
          },
        },
      ],
    },
  ],
});

describe("ReviewChatContribution", () => {
  it("keeps text-only authored contributions readable without revision actions", () => {
    const plainExtra = { review: { ...extra.review, blocks: [] } };
    expect(isRichReviewContribution(plainExtra)).toBe(false);
    render(
      <ReviewChatContribution
        extra={plainExtra}
        chatId="chat-1"
        fallback={<p>Saved authored change request</p>}
      />,
    );
    expect(screen.getByText("Saved authored change request")).toBeVisible();
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });

  it("ignores ordinary, legacy and malformed metadata", () => {
    for (const value of [
      undefined,
      {},
      { review: null },
      { review: { ...extra.review, blocks: [] } },
      { review: { ...extra.review, id: 1 } },
    ])
      expect(isRichReviewContribution(value)).toBe(false);
    expect(isRichReviewContribution(extra)).toBe(true);
  });
  it("loads only the canonical owning contribution and navigates the same thread", async () => {
    const request = vi.fn();
    server.use(
      http.get("*/v1/reviews/r1", () => {
        request();
        return HttpResponse.json(review);
      }),
    );
    const open = vi.fn();
    const { user } = render(
      <ReviewChatContribution
        extra={extra}
        chatId="chat-1"
        fallback={<p>Fallback body</p>}
        onOpenDiscussion={open}
      />,
    );
    expect(screen.getByText("Fallback body")).toBeInTheDocument();
    expect(await screen.findByText("Canonical rich reply")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Load interactive block" }),
    ).not.toBeInTheDocument();
    expect(screen.queryByText("Fallback body")).not.toBeInTheDocument();
    expect(screen.getByText(/Historical candidate/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Open discussion" }));
    expect(open).toHaveBeenCalledWith("t1");
    expect(request).toHaveBeenCalled();
  });
  it("keeps fallback for a different owning chat", async () => {
    server.use(http.get("*/v1/reviews/r1", () => HttpResponse.json(review)));
    render(
      <ReviewChatContribution
        extra={extra}
        chatId="another-chat"
        fallback={<p>Fallback body</p>}
      />,
    );
    expect(await screen.findByText("Fallback body")).toBeInTheDocument();
    expect(screen.queryByText("Canonical rich reply")).not.toBeInTheDocument();
  });
});
