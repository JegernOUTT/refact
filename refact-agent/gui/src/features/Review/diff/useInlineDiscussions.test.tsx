import { http, HttpResponse } from "msw";
import { screen, waitFor } from "@testing-library/react";
import { expect, test } from "vitest";
import { render } from "../../../utils/test-utils";
import { server } from "../../../utils/mockServer";
import { anchor, makeReview } from "../reviewFixtures";
import { useInlineDiscussions } from "./useInlineDiscussions";
import { ReviewDiffEditors } from "./ReviewDiffEditors";
import type { Review } from "../../../services/refact/review";

const first = anchor("c1", "src/a.ts", "head", 1);
const second = anchor("c1", "src/a.ts", "base", 1);
function Harness({
  path,
  review,
  two = false,
}: {
  path: string;
  review: Review;
  two?: boolean;
}) {
  const { inlineHosts, portals } = useInlineDiscussions({
    review,
    path,
    panes: {
      left: { candidateId: "c1", side: "base" },
      right: { candidateId: "c1", side: "head" },
    },
    draftAnchors: two ? [first, second] : [first],
    onCloseDraft: () => undefined,
    activeObjectId: null,
    onFocusObject: () => undefined,
  });
  return (
    <>
      <ReviewDiffEditors
        mode="side-by-side"
        left="a\nb\nc"
        right="x\ny\nz"
        path={path}
        leftLabel="Base"
        rightLabel="Head"
        highlights={{ left: [], right: [] }}
        reveal={null}
        inlineHosts={inlineHosts}
        onSelection={() => undefined}
      />
      {portals}
    </>
  );
}

test("portal drafts and unknown exact retries survive host detachment, candidate revision changes and other drafts", async () => {
  const bodies: unknown[] = [];
  server.use(
    http.post("*/v1/reviews/r1/comments", async ({ request }) => {
      bodies.push(await request.json());
      return bodies.length === 1
        ? HttpResponse.error()
        : HttpResponse.json(makeReview({ revision: 10 }));
    }),
  );
  const { user, rerender, unmount } = render(
    <Harness path="src/a.ts" review={makeReview()} />,
  );
  const input = await screen.findByLabelText("New comment on src/a.ts");
  await waitFor(() => expect(input).toHaveFocus());
  await user.type(input, "Keep exact request");
  await user.click(screen.getByRole("button", { name: "Comment" }));
  await screen.findByRole("button", { name: "Retry" });
  rerender(
    <Harness path="other.ts" review={makeReview({ revision: 8 })} two />,
  );
  expect(input.isConnected).toBe(false);
  expect(input).toHaveValue("Keep exact request");
  rerender(
    <Harness path="src/a.ts" review={makeReview({ revision: 9 })} two />,
  );
  await waitFor(() => expect(input.isConnected).toBe(true));
  expect(screen.getAllByLabelText("New comment on src/a.ts")).toHaveLength(2);
  expect(input).toHaveAttribute("readonly");
  await user.click(screen.getByRole("button", { name: "Retry" }));
  await waitFor(() => expect(input).toHaveValue(""));
  expect(bodies).toHaveLength(2);
  expect(bodies[1]).toEqual(bodies[0]);
  unmount();
  expect(input.isConnected).toBe(false);
});
