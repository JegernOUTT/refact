import { http, HttpResponse } from "msw";
import { screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type {
  Annotation,
  Contribution,
  RemoteLink,
  Review,
} from "../../services/refact/review";
import { server } from "../../utils/mockServer";
import { render } from "../../utils/test-utils";
import { ReviewDiscussion } from "./ReviewDiscussion";
import { ReviewFindings } from "./ReviewFindings";
import { ReviewRemoteContext, useReviewRemote } from "./ReviewRemote";
import { ReviewDocumentState } from "./ReviewRichBlocks";
import { setPublishComments } from "./remote";
import { anchor, makeReview } from "./reviewFixtures";

HTMLElement.prototype.scrollIntoView = () => undefined;

afterEach(() => setPublishComments(false));

function finding(
  id: string,
  severity: Annotation["severity"],
  overrides: Partial<Annotation> = {},
): Annotation {
  return {
    id,
    kind: "finding",
    title: `Finding ${id}`,
    body: `Body of ${id}`,
    anchors: [anchor("c1", "src/a.ts", "head", 4, 6)],
    severity,
    confidence: "medium",
    assessment: "unassessed",
    disposition: "open",
    applicability: "current",
    evidence_ids: [],
    run_id: null,
    ...overrides,
  };
}

function comment(
  id: string,
  actor: Contribution["actor"],
  body: string,
  at: number,
): Contribution {
  return {
    id,
    thread_id: "t1",
    actor,
    body,
    created_at_ms: at,
    chat_message_id: null,
    context: null,
  };
}

function link(
  local: string,
  state: RemoteLink["state"],
  kind: RemoteLink["kind"] = "contribution",
): RemoteLink {
  return {
    kind,
    local_id: local,
    state,
    source: "github",
    remote_id: state === "requested" ? undefined : `review_comment:${local}`,
    url: state === "requested" ? undefined : `https://example.test/${local}`,
    at_ms: 1,
  };
}

/** Mounts children under the live PR context of one review. */
function Canvas({
  review,
  children,
}: {
  review: Review;
  children: React.ReactNode;
}) {
  const remote = useReviewRemote(review.id, true);
  return (
    <ReviewRemoteContext.Provider value={remote}>
      <ReviewDocumentState>{children}</ReviewDocumentState>
    </ReviewRemoteContext.Provider>
  );
}

function servePr(report: Record<string, unknown> = {}) {
  const requests: { path: string; body: unknown }[] = [];
  server.use(
    http.post("*/v1/reviews/r1/pr-sync", async ({ request }) => {
      requests.push({ path: "pr-sync", body: await request.json() });
      return HttpResponse.json({
        available: true,
        pull_request: "acme/widgets#12",
        imported: 0,
        published: 0,
        ...report,
      });
    }),
    http.post("*/v1/reviews/r1/outbox", async ({ request }) => {
      requests.push({ path: "outbox", body: await request.json() });
      return HttpResponse.json(makeReview());
    }),
    http.post("*/v1/reviews/r1/comments", async ({ request }) => {
      requests.push({ path: "comments", body: await request.json() });
      return HttpResponse.json(makeReview());
    }),
    http.get("*/v1/reviews/r1/delivery-status", () => HttpResponse.json([])),
    http.get("*/v1/reviews/r1/history", () => HttpResponse.json([])),
  );
  return requests;
}

describe("findings view", () => {
  it("lists every finding most severe first, names its file and settles dismissed ones", async () => {
    const review = makeReview({
      annotations: [
        finding("low", "low"),
        finding("gone", "critical", { disposition: "dismissed" }),
        finding("crit", "critical"),
        finding("note", "info", { kind: "explanation" }),
      ],
    });
    const onShow = vi.fn();
    const { user } = render(
      <ReviewDocumentState>
        <ReviewFindings
          review={review}
          activeObjectId={null}
          working={false}
          onShow={onShow}
        />
      </ReviewDocumentState>,
    );
    const open = screen.getByRole("region", { name: "Open findings" });
    expect(
      within(open)
        .getAllByRole("article")
        .map((item) => item.getAttribute("aria-label")),
    ).toEqual(["Finding: Finding crit", "Finding: Finding low"]);
    expect(
      within(
        screen.getByRole("region", { name: "Settled findings" }),
      ).getByRole("article"),
    ).toHaveAccessibleName("Finding: Finding gone");
    expect(
      within(screen.getByRole("region", { name: "Notes findings" })).getByRole(
        "article",
      ),
    ).toHaveAccessibleName("Explanation: Finding note");
    await user.click(
      within(open).getAllByRole("button", { name: "src/a.ts:4-6" })[0],
    );
    expect(onShow).toHaveBeenCalledWith("crit");
    // No pull request is known here, so nothing offers to send.
    expect(
      screen.queryByRole("button", { name: /Send to PR/ }),
    ).not.toBeInTheDocument();
  });

  it("says findings are still coming while the review works", () => {
    render(
      <ReviewFindings
        review={makeReview()}
        activeObjectId={null}
        working
        onShow={vi.fn()}
      />,
    );
    expect(screen.getByText(/No findings yet/)).toBeInTheDocument();
  });
});

describe("pull request round trip", () => {
  it("syncs quietly when the canvas opens and sends a finding once on request", async () => {
    const requests = servePr();
    const review = makeReview({ annotations: [finding("f1", "high")] });
    const { user } = render(
      <Canvas review={review}>
        <ReviewFindings
          review={review}
          activeObjectId={null}
          working={false}
          onShow={vi.fn()}
        />
      </Canvas>,
    );
    await user.click(await screen.findByRole("button", { name: /Send to PR/ }));
    await waitFor(() =>
      expect(requests).toEqual([
        { path: "pr-sync", body: { quiet: true } },
        { path: "outbox", body: { id: "f1" } },
      ]),
    );
  });

  it("shows whose comment it is, where it stands with the PR, and syncs on demand", async () => {
    const requests = servePr();
    const review = makeReview({
      threads: [
        {
          id: "t1",
          annotation_id: null,
          anchors: [anchor("c1", "src/a.ts", "head", 2)],
          resolved: false,
          contributions: [
            comment(
              "c-alice",
              { kind: "external", id: "github:alice" },
              "Why?",
              1,
            ),
            comment("c-sent", { kind: "user", id: "user" }, "Because.", 2),
            comment("c-wait", { kind: "user", id: "user" }, "Also this.", 3),
            comment("c-local", { kind: "agent", id: "chat-1" }, "Checked.", 4),
          ],
        },
      ],
      remote_links: [
        link("c-alice", "imported"),
        link("c-sent", "published"),
        link("c-wait", "requested"),
      ],
    });
    const onShow = vi.fn();
    const { user } = render(
      <Canvas review={review}>
        <ReviewDiscussion
          review={review}
          activeObjectId={null}
          onShow={onShow}
        />
      </Canvas>,
    );
    expect(
      await screen.findByText("Pull request acme/widgets#12"),
    ).toBeInTheDocument();
    expect(screen.getByText("alice")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /From the PR/ })).toHaveAttribute(
      "href",
      "https://example.test/c-alice",
    );
    expect(screen.getByRole("link", { name: /On the PR/ })).toHaveAttribute(
      "href",
      "https://example.test/c-sent",
    );
    expect(screen.getByText("Waiting to send")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Send again/ })).toBeEnabled();
    // The agent's unsent reply can be sent; other people's comments cannot.
    expect(screen.getAllByRole("button", { name: /Send to PR/ })).toHaveLength(
      1,
    );

    await user.click(screen.getByRole("button", { name: "src/a.ts:2" }));
    expect(onShow).toHaveBeenCalledWith(review.threads[0]);
    await user.click(screen.getByRole("button", { name: /Sync now/ }));
    await waitFor(() =>
      expect(requests.at(-1)).toEqual({
        path: "pr-sync",
        body: { quiet: false },
      }),
    );
  });

  it("posts a new comment to the PR only when the user chose that", async () => {
    const requests = servePr();
    const review = makeReview({
      threads: [
        {
          id: "t1",
          annotation_id: null,
          anchors: [anchor("c1", "src/a.ts", "head", 2)],
          resolved: false,
          contributions: [
            comment(
              "c-alice",
              { kind: "external", id: "github:alice" },
              "Why?",
              1,
            ),
          ],
        },
      ],
      remote_links: [link("c-alice", "imported")],
    });
    const { user } = render(
      <Canvas review={review}>
        <ReviewDiscussion
          review={review}
          activeObjectId={null}
          onShow={vi.fn()}
        />
      </Canvas>,
    );
    await screen.findByText("Pull request acme/widgets#12");
    const sent = () =>
      requests
        .filter((request) => request.path === "comments")
        .map((request) => request.body as Record<string, unknown>);

    await user.click(screen.getByRole("button", { name: /Reply/ }));
    await user.type(screen.getByRole("textbox", { name: "Reply" }), "First");
    await user.click(screen.getByRole("button", { name: "Comment" }));
    await waitFor(() => expect(sent()).toHaveLength(1));
    expect(sent()[0]).not.toHaveProperty("publish");

    await user.click(screen.getByRole("button", { name: /Reply/ }));
    await user.click(
      screen.getByRole("checkbox", { name: "Also post to acme/widgets#12" }),
    );
    expect(
      screen.getByRole("checkbox", { name: "Post my comments to the PR" }),
    ).toBeChecked();
    await user.type(screen.getByRole("textbox", { name: "Reply" }), "Second");
    await user.click(screen.getByRole("button", { name: "Comment" }));
    await waitFor(() => expect(sent()).toHaveLength(2));
    expect(sent()[1]).toMatchObject({ body: "Second", publish: true });
  });

  it("offers nothing for the PR when the review has none", async () => {
    const review = makeReview({ annotations: [finding("f1", "high")] });
    render(
      <Canvas review={review}>
        <ReviewDiscussion
          review={review}
          activeObjectId={null}
          onShow={vi.fn()}
        />
        <ReviewFindings
          review={review}
          activeObjectId={null}
          working={false}
          onShow={vi.fn()}
        />
      </Canvas>,
    );
    expect(await screen.findByText(/No discussion yet/)).toBeInTheDocument();
    expect(screen.queryByText(/Pull request/)).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /Send to PR|Sync now/ }),
    ).not.toBeInTheDocument();
  });
});
