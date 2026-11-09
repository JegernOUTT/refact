import type {
  Anchor,
  Candidate,
  Review,
  ReviewSide,
} from "../../services/refact/review";

export function anchor(
  candidateId: string,
  path: string,
  side: ReviewSide,
  start: number,
  end = start,
): Anchor {
  return {
    position: {
      candidate_id: candidateId,
      path,
      side,
      start_line: start,
      end_line: end,
      content_hash: `h-${candidateId}-${start}`,
      context_before: [],
      context_after: [],
    },
    status: "current",
    stalled_reason: null,
    history: [],
  };
}

export function candidate(
  id: string,
  createdAt: number,
  files: Candidate["files"],
): Candidate {
  return {
    id,
    base_oid: `base${id}0000000`,
    head_oid: `head${id}0000000`,
    content_digest: `d-${id}`,
    created_at_ms: createdAt,
    repository: "/repo",
    origin: "local",
    policy: { staged: true, working: true, untracked: false },
    files,
  };
}

export function makeReview(overrides: Partial<Review> = {}): Review {
  const c1 = candidate("c1", 1, [
    {
      path: "src/a.ts",
      base: { digest: "a0", size: 1 },
      head: { digest: "a1", size: 1 },
      base_mode: 0o100644,
      head_mode: 0o100644,
    },
    {
      path: "src/same.ts",
      base: { digest: "s", size: 1 },
      head: { digest: "s", size: 1 },
      base_mode: 0o100644,
      head_mode: 0o100644,
    },
  ]);
  return {
    schema_version: 1,
    id: "r1",
    owner_chat_id: "chat-1",
    revision: 3,
    current_candidate_id: "c1",
    title: "Review of feature",
    origin: "local",
    target_reference: "main",
    candidates: [c1],
    tour: [],
    annotations: [],
    threads: [],
    evidence: [],
    runs: [],
    materials: [],
    activity: [],
    ...overrides,
  };
}

export function bytes(text: string): number[] {
  return Array.from(new TextEncoder().encode(text));
}
