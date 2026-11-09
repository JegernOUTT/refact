import { http, HttpResponse } from "msw";
import { screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { server } from "../../utils/mockServer";
import { render } from "../../utils/test-utils";
import { CommentComposer } from "./CommentComposer";
import { makeReview } from "./reviewFixtures";

type Body = Record<string, unknown>;

function captureComments(
  responses: ((body: Body) => Response)[],
  bodies: Body[],
) {
  server.use(
    http.post("*/v1/reviews/r1/comments", async ({ request }) => {
      const body = (await request.json()) as Body;
      bodies.push(body);
      const respond = responses.shift();
      if (!respond) throw new Error("unexpected comment request");
      return respond(body);
    }),
  );
}

describe("CommentComposer", () => {
  it("keeps the draft and issues a fresh key after a revision conflict", async () => {
    const bodies: Body[] = [];
    captureComments(
      [
        () =>
          HttpResponse.json(
            { detail: "revision conflict: expected 3, actual 4" },
            { status: 409 },
          ),
        () => HttpResponse.json(makeReview({ revision: 5 })),
      ],
      bodies,
    );
    const { user, rerender } = render(
      <CommentComposer
        reviewId="r1"
        revision={3}
        target={{ kind: "reply", threadId: "t1" }}
        label="Reply"
      />,
    );
    const input = screen.getByLabelText("Reply");
    await user.type(input, "Looks racy");
    await user.click(screen.getByRole("button", { name: "Comment" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      /review changed/i,
    );
    expect(input).toHaveValue("Looks racy");

    rerender(
      <CommentComposer
        reviewId="r1"
        revision={4}
        target={{ kind: "reply", threadId: "t1" }}
        label="Reply"
      />,
    );
    await user.click(screen.getByRole("button", { name: "Comment" }));
    await waitFor(() => expect(input).toHaveValue(""));

    expect(bodies).toHaveLength(2);
    expect(bodies[0].expected_revision).toBe(3);
    expect(bodies[1].expected_revision).toBe(4);
    expect(bodies[1].request_key).not.toBe(bodies[0].request_key);
    expect(bodies[1].thread_id).toBe("t1");
  });

  it("retries an unacknowledged comment with the identical request even after the review moves", async () => {
    const bodies: Body[] = [];
    captureComments(
      [
        () => HttpResponse.error(),
        () => HttpResponse.json(makeReview({ revision: 9 })),
      ],
      bodies,
    );
    const { user, rerender } = render(
      <CommentComposer
        reviewId="r1"
        revision={3}
        target={{ kind: "reply", threadId: "t1" }}
        label="Reply"
      />,
    );
    const input = screen.getByLabelText("Reply");
    await user.type(input, "First take");
    await user.click(screen.getByRole("button", { name: "Comment" }));

    const retry = await screen.findByRole("button", { name: "Retry" });
    expect(input).toHaveAttribute("readonly");
    expect(
      screen.queryByRole("button", { name: /edit instead/i }),
    ).not.toBeInTheDocument();

    // An unrelated worker advances the review while the outcome is unknown.
    rerender(
      <CommentComposer
        reviewId="r1"
        revision={8}
        target={{ kind: "reply", threadId: "t1" }}
        label="Reply"
      />,
    );
    await user.type(input, " edited");
    await user.click(retry);
    await waitFor(() => expect(input).toHaveValue(""));

    expect(bodies).toHaveLength(2);
    expect(bodies[1]).toEqual(bodies[0]);
    expect(bodies[1].expected_revision).toBe(3);
    expect(bodies[1].body).toBe("First take");
  });
});
