import { http, HttpResponse } from "msw";
import { screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { server } from "../../utils/mockServer";
import { render } from "../../utils/test-utils";
import type { Annotation, ObjectRevision } from "../../services/refact/review";
import { ObjectHistory } from "./ObjectHistory";
import { anchor } from "./reviewFixtures";

function annotation(overrides: Partial<Annotation>): Annotation {
  return {
    id: "an1",
    kind: "finding",
    title: "Race in cache",
    body: "Current wording.",
    anchors: [anchor("c2", "src/a.ts", "head", 3)],
    severity: "high",
    confidence: "medium",
    assessment: "supported",
    disposition: "fixed",
    applicability: "current",
    evidence_ids: [],
    run_id: null,
    ...overrides,
  };
}

describe("ObjectHistory", () => {
  it("fetches on demand and shows prior claims with provenance, newest first", async () => {
    let requests = 0;
    const current = annotation({});
    const revisions: ObjectRevision[] = [
      {
        review_id: "r1",
        revision: 2,
        actor: { kind: "agent", id: "a" },
        at_ms: 1000,
        object: {
          kind: "annotation",
          value: annotation({
            body: "Original claim: unbounded growth.",
            disposition: "open",
            anchors: [anchor("c1", "src/a.ts", "head", 3)],
          }),
        },
      },
      {
        review_id: "r1",
        revision: 5,
        actor: { kind: "user", id: "u" },
        at_ms: 2000,
        object: {
          kind: "annotation",
          value: annotation({ body: "Second claim.", disposition: "open" }),
        },
      },
      {
        review_id: "r1",
        revision: 7,
        actor: { kind: "agent", id: "a" },
        at_ms: 3000,
        object: { kind: "annotation", value: current },
      },
      {
        review_id: "r1",
        revision: 4,
        actor: { kind: "agent", id: "a" },
        at_ms: 1500,
        object: { kind: "annotation", value: annotation({ id: "other" }) },
      },
    ];
    server.use(
      http.get("*/v1/reviews/r1/history", () => {
        requests += 1;
        return HttpResponse.json(revisions);
      }),
    );
    const { user } = render(
      <ObjectHistory
        reviewId="r1"
        current={{ kind: "annotation", value: current }}
      />,
    );
    expect(requests).toBe(0);
    await user.click(screen.getByRole("button", { name: "History" }));

    expect(
      await screen.findByText("Original claim: unbounded growth."),
    ).toBeInTheDocument();
    const items = screen.getAllByLabelText(/^Revision \d+$/);
    expect(items.map((el) => el.getAttribute("aria-label"))).toEqual([
      "Revision 5",
      "Revision 2",
    ]);
    expect(items[1]).toHaveTextContent(/Agent/);
    expect(items[1]).toHaveTextContent(/candidate c1/);
    expect(screen.queryByText("Current wording.")).not.toBeInTheDocument();
    expect(requests).toBe(1);
    const toggle = screen.getByRole("button", { name: "History" });
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    await user.click(toggle);
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByText("Second claim.")).not.toBeInTheDocument();
    await user.click(toggle);
    expect(await screen.findByText("Second claim.")).toBeInTheDocument();
  });
});
