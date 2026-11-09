import { describe, expect, it } from "vitest";

import { decodeFileSide } from "../../services/refact/review";
import {
  diffPanes,
  fileStatus,
  paneFor,
  reattachOperation,
  reviewFiles,
  stalledTargets,
} from "./model";
import { anchor, candidate, makeReview } from "./reviewFixtures";

describe("review model", () => {
  it("lists changed files first and hides unchanged unless requested", () => {
    const review = makeReview();
    const c = review.candidates[0];
    expect(reviewFiles(review, c, "full", false).map((f) => f.path)).toEqual([
      "src/a.ts",
    ]);
    expect(reviewFiles(review, c, "full", true).map((f) => f.status)).toEqual([
      "modified",
      "unchanged",
    ]);
  });

  it("treats a mode-only change as modified", () => {
    expect(
      fileStatus({
        path: "run.sh",
        base: { digest: "x", size: 1 },
        head: { digest: "x", size: 1 },
        base_mode: 0o100644,
        head_mode: 0o100755,
      }),
    ).toBe("modified");
  });

  it("since-last compares previous head and keeps removed files as deletions", () => {
    const c1 = candidate("c1", 1, [
      {
        path: "keep.ts",
        base: null,
        head: { digest: "k1", size: 1 },
        base_mode: null,
        head_mode: 1,
      },
      {
        path: "gone.ts",
        base: null,
        head: { digest: "g", size: 1 },
        base_mode: null,
        head_mode: 1,
      },
    ]);
    const c2 = candidate("c2", 2, [
      {
        path: "keep.ts",
        base: null,
        head: { digest: "k1", size: 1 },
        base_mode: null,
        head_mode: 1,
      },
    ]);
    const review = makeReview({
      candidates: [c1, c2],
      current_candidate_id: "c2",
    });
    expect(reviewFiles(review, c2, "full", false)).toHaveLength(1);
    const sinceLast = reviewFiles(review, c2, "since_last", false);
    expect(sinceLast.map((f) => [f.path, f.status])).toEqual([
      ["gone.ts", "deleted"],
    ]);
  });

  it("maps since-last left pane to the previous candidate head", () => {
    const c1 = candidate("c1", 1, []);
    const c2 = candidate("c2", 2, []);
    const full = diffPanes(c2, c1, "full");
    expect(full.left).toEqual({ candidateId: "c2", side: "base" });
    const since = diffPanes(c2, c1, "since_last");
    expect(since.left).toEqual({ candidateId: "c1", side: "head" });
    expect(since.right).toEqual({ candidateId: "c2", side: "head" });
    expect(paneFor(since, { candidate_id: "c1", side: "head" })).toBe("left");
    expect(paneFor(since, { candidate_id: "c2", side: "base" })).toBeNull();
  });

  it("reattaches a stalled thread anchor preserving position history", () => {
    const stalled = {
      ...anchor("c1", "src/a.ts", "head", 4),
      status: "stalled" as const,
      stalled_reason: "no exact contextual match",
    };
    const review = makeReview({
      threads: [
        {
          id: "t1",
          anchors: [stalled],
          annotation_id: null,
          resolved: false,
          contributions: [],
        },
      ],
    });
    const [target] = stalledTargets(review);
    expect(target.kind).toBe("thread");
    const fresh = anchor("c1", "src/a.ts", "head", 9);
    const op = reattachOperation(review, target, fresh);
    expect(op?.op).toBe("put_thread");
    if (op?.op !== "put_thread") return;
    expect(op.thread.anchors[0].position.start_line).toBe(9);
    expect(op.thread.anchors[0].status).toBe("current");
    expect(op.thread.anchors[0].history).toEqual([stalled.position]);
  });

  it("decodes strictly and reports invalid UTF-8 as binary", () => {
    expect(decodeFileSide(null)).toEqual({ kind: "absent" });
    expect(decodeFileSide([104, 105])).toEqual({ kind: "text", text: "hi" });
    expect(decodeFileSide([0xff, 0xfe, 0x00])).toEqual({
      kind: "binary",
      size: 3,
    });
  });
});
