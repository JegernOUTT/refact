import { screen, waitFor } from "@testing-library/react";
import { isReview } from "../../services/refact/review";
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

const target = { candidate_id: "c1", step_id: "s1", version: 1 };
const proposal = {
  id: "p1",
  target,
  thread_id: "t1",
  proposal_contribution_id: "c1",
  change_request: "Explain ordering",
  proposed_by: { kind: "agent" as const, id: "agent" },
  proposed_at_ms: 1,
  acceptance: null,
  fulfilment: null,
};
const proposalExtra = {
  review: { ...extra.review, blocks: [], tour_revision_proposal_id: "p1" },
};
const proposalReview = makeReview({
  threads: [
    {
      ...review.threads[0],
      contributions: [
        {
          ...review.threads[0].contributions[0],
          blocks: [],
          body: "Canonical proposal",
        },
      ],
    },
  ],
  tour: [
    {
      id: "s1",
      current: {
        candidate_id: "c1",
        version: 1,
        title: "Step",
        body: "",
        anchors: [],
      },
      history: [],
    },
  ],
  tour_revision_proposals: [proposal],
});

describe("ReviewChatContribution", () => {
  const acceptance = {
    actor: { kind: "user" as const, id: "gui" },
    accepted_at_ms: 2,
    contribution_id: "receipt",
  };

  it.each(["fulfilled", "candidate drift", "version drift"])(
    "renders durable %s without offering acceptance",
    async (state) => {
      const current = {
        ...proposalReview,
        current_candidate_id: state === "candidate drift" ? "other" : "c1",
        tour: proposalReview.tour.map((step) => ({
          ...step,
          current: {
            ...step.current,
            version: state === "candidate drift" ? 1 : 2,
          },
        })),
        tour_revision_proposals: [
          {
            ...proposal,
            acceptance,
            fulfilment:
              state === "fulfilled"
                ? {
                    actor: { kind: "agent" as const, id: "agent" },
                    fulfilled_at_ms: 3,
                    version: 2,
                  }
                : null,
          },
        ],
      };
      const post = vi.fn();
      server.use(
        http.get("*/v1/reviews/r1", () => HttpResponse.json(current)),
        http.post("*", () => {
          post();
          return HttpResponse.json({});
        }),
      );
      render(
        <ReviewChatContribution
          extra={proposalExtra}
          chatId="chat-1"
          fallback="fallback"
        />,
      );
      expect(
        await screen.findByText(
          state === "fulfilled"
            ? "Fulfilled"
            : "Accepted pending · Stale target",
        ),
      ).toBeInTheDocument();
      expect(
        screen.queryByRole("button", { name: "Accept Tour revision" }),
      ).not.toBeInTheDocument();
      expect(screen.queryByText("Stale")).not.toBeInTheDocument();
      if (state === "fulfilled") {
        expect(
          screen.queryByText(/Ask Review to continue/),
        ).not.toBeInTheDocument();
        expect(screen.queryByText(/Accepted pending/)).not.toBeInTheDocument();
      } else
        expect(screen.getByText(/Ask Review to continue/)).toBeInTheDocument();
      expect(post).not.toHaveBeenCalled();
    },
  );

  it.each([false, true])(
    "requires manual retry after a lost response (canonical acceptance: %s)",
    async (accepted) => {
      let current = proposalReview;
      const post = vi.fn();
      const get = vi.fn();
      server.use(
        http.get("*/v1/reviews/r1", () => {
          get();
          return HttpResponse.json(current);
        }),
        http.post(
          "*/v1/reviews/r1/tour-revision-proposals/p1/accept",
          async ({ request }) => {
            post(await request.json());
            current = {
              ...proposalReview,
              revision: 7,
              tour_revision_proposals: [
                { ...proposal, acceptance: accepted ? acceptance : null },
              ],
            };
            return HttpResponse.error();
          },
        ),
      );
      const { user } = render(
        <ReviewChatContribution
          extra={proposalExtra}
          chatId="chat-1"
          fallback="fallback"
        />,
      );
      await user.click(
        await screen.findByRole("button", { name: "Accept Tour revision" }),
      );
      if (!accepted)
        expect(
          await screen.findByText(/Acceptance could not be confirmed/),
        ).toBeInTheDocument();
      await waitFor(() => expect(get).toHaveBeenCalledTimes(2));
      if (accepted) {
        expect(await screen.findByText("Accepted pending")).toBeInTheDocument();
        expect(
          screen.queryByText(/Acceptance could not be confirmed/),
        ).not.toBeInTheDocument();
        expect(
          screen.queryByRole("button", { name: "Accept Tour revision" }),
        ).not.toBeInTheDocument();
        expect(post).toHaveBeenCalledTimes(1);
      } else {
        const retry = screen.getByRole("button", {
          name: "Accept Tour revision",
        });
        await waitFor(() => expect(retry).toBeEnabled());
        expect(post).toHaveBeenCalledTimes(1);
        await user.click(retry);
        await waitFor(() => expect(post).toHaveBeenCalledTimes(2));
        expect(post.mock.calls[0][0]).toEqual(
          expect.objectContaining({
            expected_revision: 3,
            request_key: expect.any(String) as unknown,
            target,
          }),
        );
        expect(post.mock.calls[1][0]).toEqual(post.mock.calls[0][0]);
        await waitFor(() => expect(get).toHaveBeenCalledTimes(3));
      }
    },
  );

  it.each([
    {
      locator: { ...proposalExtra.review, tour_revision_proposal_id: "forged" },
      chatId: "chat-1",
    },
    {
      locator: { ...proposalExtra.review, thread_id: "forged" },
      chatId: "chat-1",
    },
    {
      locator: { ...proposalExtra.review, tour_revision_proposal_id: 123 },
      chatId: "chat-1",
    },
    { locator: proposalExtra.review, chatId: "another-chat" },
  ])(
    "does not grant authority to $chatId with locator $locator",
    async ({ locator, chatId }) => {
      const post = vi.fn();
      server.use(
        http.get("*/v1/reviews/r1", () => HttpResponse.json(proposalReview)),
        http.post("*", () => {
          post();
          return HttpResponse.json({});
        }),
      );
      render(
        <ReviewChatContribution
          extra={{ review: locator }}
          chatId={chatId}
          fallback="fallback"
        />,
      );
      await waitFor(() =>
        expect(
          screen.queryByText("Loading Review contribution…"),
        ).not.toBeInTheDocument(),
      );
      expect(screen.getByText("fallback")).toBeInTheDocument();
      expect(
        screen.queryByRole("button", { name: "Accept Tour revision" }),
      ).not.toBeInTheDocument();
      expect(post).not.toHaveBeenCalled();
    },
  );

  it("validates proposal wire fields rather than trusting truthy receipt objects", () => {
    expect(isReview(proposalReview)).toBe(true);
    expect(
      isReview({
        ...proposalReview,
        tour_revision_proposals: [
          {
            ...proposal,
            acceptance,
            fulfilment: {
              actor: proposal.proposed_by,
              fulfilled_at_ms: 3,
              version: 2,
            },
          },
        ],
      }),
    ).toBe(true);
    for (const patch of [
      { target: null },
      { target: { ...target, version: "1" } },
      { target: { ...target, version: -1 } },
      { target: { ...target, version: 1.5 } },
      { target: { ...target, block_revision: "1" } },
      { target: { ...target, block_id: [] } },
      { proposed_by: { kind: "attacker", id: "x" } },
      { proposed_at_ms: Infinity },
      { change_request: {} },
      { proposal_contribution_id: 1 },
      { acceptance: {} },
      { acceptance: { ...acceptance, actor: null } },
      { acceptance: { ...acceptance, accepted_at_ms: "2" } },
      { acceptance: { ...acceptance, contribution_id: [] } },
      {
        fulfilment: {
          actor: proposal.proposed_by,
          fulfilled_at_ms: 3,
          version: "2",
        },
      },
      { fulfilment: true },
    ])
      expect(
        isReview({
          ...proposalReview,
          tour_revision_proposals: [{ ...proposal, ...patch }],
        }),
        JSON.stringify(patch),
      ).toBe(false);
  });

  it("accepts a canonical text proposal and rereads durable pending status", async () => {
    let current = proposalReview;
    const post = vi.fn();
    server.use(
      http.get("*/v1/reviews/r1", () => HttpResponse.json(current)),
      http.post(
        "*/v1/reviews/r1/tour-revision-proposals/p1/accept",
        async ({ request }) => {
          post(await request.json());
          current = {
            ...proposalReview,
            revision: 4,
            tour_revision_proposals: [
              {
                ...proposal,
                acceptance: {
                  actor: { kind: "user", id: "gui" },
                  accepted_at_ms: 2,
                  contribution_id: "receipt",
                },
              },
            ],
          };
          return HttpResponse.json(current);
        },
      ),
    );
    expect(isRichReviewContribution(proposalExtra)).toBe(true);
    const { user, unmount } = render(
      <ReviewChatContribution
        extra={proposalExtra}
        chatId="chat-1"
        fallback="fallback"
      />,
    );
    expect(await screen.findByText("Canonical proposal")).toBeInTheDocument();
    await user.click(
      await screen.findByRole("button", { name: "Accept Tour revision" }),
    );
    expect(await screen.findByText("Accepted pending")).toBeInTheDocument();
    expect(screen.getByText(/Ask Review to continue/)).toBeInTheDocument();
    expect(post).toHaveBeenCalledTimes(1);
    expect(post).toHaveBeenCalledWith(
      expect.objectContaining({
        owner_chat_id: "chat-1",
        expected_revision: 3,
        target,
      }),
    );
    unmount();
    render(
      <ReviewChatContribution
        extra={proposalExtra}
        chatId="chat-1"
        fallback="fallback"
      />,
    );
    expect(await screen.findByText("Accepted pending")).toBeInTheDocument();
  });
  it("rejects forged locators and disables stale proposals", async () => {
    server.use(
      http.get("*/v1/reviews/r1", () =>
        HttpResponse.json({ ...proposalReview, current_candidate_id: "other" }),
      ),
    );
    const { unmount } = render(
      <ReviewChatContribution
        extra={proposalExtra}
        chatId="chat-1"
        fallback="fallback"
      />,
    );
    expect(await screen.findByText("Stale")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Accept Tour revision" }),
    ).toBeDisabled();
    unmount();
    render(
      <ReviewChatContribution
        extra={{
          review: { ...proposalExtra.review, contribution_id: "forged" },
        }}
        chatId="chat-1"
        fallback="fallback"
      />,
    );
    expect(await screen.findByText("fallback")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Accept Tour revision" }),
    ).not.toBeInTheDocument();
  });
  it("refetches after conflict without automatic resubmission", async () => {
    const post = vi.fn();
    const get = vi.fn();
    server.use(
      http.get("*/v1/reviews/r1", () => {
        get();
        return HttpResponse.json(proposalReview);
      }),
      http.post("*/v1/reviews/r1/tour-revision-proposals/p1/accept", () => {
        post();
        return HttpResponse.json({}, { status: 409 });
      }),
    );
    const { user } = render(
      <ReviewChatContribution
        extra={proposalExtra}
        chatId="chat-1"
        fallback="fallback"
      />,
    );
    await user.click(
      await screen.findByRole("button", { name: "Accept Tour revision" }),
    );
    expect(await screen.findByText(/Review changed/)).toBeInTheDocument();
    expect(post).toHaveBeenCalledTimes(1);
    expect(get.mock.calls.length).toBeGreaterThan(1);
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
