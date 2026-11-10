import type {
  Anchor,
  Annotation,
  Candidate,
  CandidateFile,
  HistoricalObject,
  ObjectRevision,
  Review,
  ReviewOperation,
  ReviewSide,
  ReviewThread,
  TourStep,
} from "../../services/refact/review";

export type FileStatus = "added" | "deleted" | "modified" | "unchanged";
export type CompareMode = "full" | "since_last";

export function objectAnchor(review: Review, objectId: string): Anchor | null {
  const annotation = review.annotations.find((a) => a.id === objectId);
  if (annotation) return firstAnchorOf(annotation.anchors);
  const thread = review.threads.find((t) => t.id === objectId);
  if (thread) return firstAnchorOf(thread.anchors);
  const step = review.tour.find((s) => s.id === objectId);
  if (step) return firstAnchorOf(step.current.anchors);
  return null;
}

export type ReviewFileEntry = {
  path: string;
  status: FileStatus;
  threadCount: number;
  findingCount: number;
};

export function groupReviewFiles(entries: ReviewFileEntry[]): {
  changed: ReviewFileEntry[];
  unchanged: ReviewFileEntry[];
} {
  return {
    changed: entries.filter((entry) => entry.status !== "unchanged"),
    unchanged: entries.filter((entry) => entry.status === "unchanged"),
  };
}

export function fileStatus(file: CandidateFile): FileStatus {
  if (!file.base && file.head) return "added";
  if (file.base && !file.head) return "deleted";
  if (
    file.base?.digest === file.head?.digest &&
    file.base_mode === file.head_mode
  ) {
    return "unchanged";
  }
  return "modified";
}

/** Candidates in capture order (oldest first). */
export function orderedCandidates(review: Review): Candidate[] {
  return [...review.candidates].sort(
    (a, b) => a.created_at_ms - b.created_at_ms,
  );
}

export function findCandidate(
  review: Review,
  candidateId: string | null,
): Candidate | null {
  if (!candidateId) return null;
  return review.candidates.find((c) => c.id === candidateId) ?? null;
}

export function previousCandidate(
  review: Review,
  candidateId: string,
): Candidate | null {
  const ordered = orderedCandidates(review);
  const index = ordered.findIndex((c) => c.id === candidateId);
  return index > 0 ? ordered[index - 1] : null;
}

/** Anchors attached to a review object positioned in a given candidate/path. */
export function anchorsIn(
  anchors: Anchor[],
  candidateId: string,
  path: string,
): Anchor[] {
  return anchors.filter(
    (a) => a.position.candidate_id === candidateId && a.position.path === path,
  );
}

function countBy<T>(items: T[], anchorsOf: (item: T) => Anchor[]) {
  return (candidateId: string, path: string) =>
    items.filter((item) => anchorsIn(anchorsOf(item), candidateId, path).length)
      .length;
}

/**
 * File list for a candidate. In since-last mode the status compares the
 * previous candidate's head with this candidate's head. Changed files come
 * first; unchanged files are only included when requested.
 */
export function reviewFiles(
  review: Review,
  candidate: Candidate,
  mode: CompareMode,
  includeUnchanged: boolean,
): ReviewFileEntry[] {
  const previous =
    mode === "since_last" ? previousCandidate(review, candidate.id) : null;
  const threadCount = countBy(review.threads, (t) => t.anchors);
  const findingCount = countBy(
    review.annotations.filter((a) => a.kind === "finding"),
    (a) => a.anchors,
  );
  const entries = candidate.files.map((file): ReviewFileEntry => {
    let status = fileStatus(file);
    if (previous) {
      const before = previous.files.find((f) => f.path === file.path);
      status = fileStatus({
        path: file.path,
        base: before?.head ?? null,
        head: file.head,
        base_mode: before?.head_mode ?? null,
        head_mode: file.head_mode,
      });
    }
    return {
      path: file.path,
      status,
      threadCount: threadCount(candidate.id, file.path),
      findingCount: findingCount(candidate.id, file.path),
    };
  });
  if (previous) {
    for (const before of previous.files) {
      if (candidate.files.some((f) => f.path === before.path)) continue;
      if (!before.head) continue;
      entries.push({
        path: before.path,
        status: "deleted",
        threadCount: 0,
        findingCount: 0,
      });
    }
  }
  const changed = entries.filter((e) => e.status !== "unchanged");
  const unchanged = entries.filter((e) => e.status === "unchanged");
  const byPath = (a: ReviewFileEntry, b: ReviewFileEntry) =>
    a.path.localeCompare(b.path);
  changed.sort(byPath);
  unchanged.sort(byPath);
  return includeUnchanged ? [...changed, ...unchanged] : changed;
}

export type StalledTarget =
  | { kind: "thread"; id: string; index: number; anchor: Anchor; label: string }
  | {
      kind: "annotation";
      id: string;
      index: number;
      anchor: Anchor;
      label: string;
    }
  | { kind: "tour"; id: string; index: number; anchor: Anchor; label: string };

export function stalledTargets(review: Review): StalledTarget[] {
  const out: StalledTarget[] = [];
  review.threads.forEach((thread) =>
    thread.anchors.forEach((anchor, index) => {
      if (anchor.status !== "stalled") return;
      const first = thread.contributions.at(0);
      out.push({
        kind: "thread",
        id: thread.id,
        index,
        anchor,
        label: first ? first.body.slice(0, 80) : "Comment thread",
      });
    }),
  );
  review.annotations.forEach((annotation) =>
    annotation.anchors.forEach((anchor, index) => {
      if (anchor.status !== "stalled") return;
      out.push({
        kind: "annotation",
        id: annotation.id,
        index,
        anchor,
        label: annotation.title,
      });
    }),
  );
  review.tour.forEach((step) =>
    step.current.anchors.forEach((anchor, index) => {
      if (anchor.status !== "stalled") return;
      out.push({
        kind: "tour",
        id: step.id,
        index,
        anchor,
        label: step.current.title,
      });
    }),
  );
  return out;
}

/** Preserve provenance: the replaced position is appended to history. */
export function reattachedAnchor(previous: Anchor, fresh: Anchor): Anchor {
  return {
    ...fresh,
    history: [...previous.history, previous.position],
  };
}

function replaceAt(anchors: Anchor[], index: number, next: Anchor): Anchor[] {
  return anchors.map((a, i) => (i === index ? reattachedAnchor(a, next) : a));
}

/** Build the put operation that reattaches a stalled anchor, or null if gone. */
export function reattachOperation(
  review: Review,
  target: StalledTarget,
  fresh: Anchor,
): ReviewOperation | null {
  if (target.kind === "thread") {
    const thread = review.threads.find((t) => t.id === target.id);
    if (target.index >= (thread?.anchors.length ?? 0) || !thread) return null;
    const next: ReviewThread = {
      ...thread,
      anchors: replaceAt(thread.anchors, target.index, fresh),
    };
    return { op: "put_thread", thread: next };
  }
  if (target.kind === "annotation") {
    const annotation = review.annotations.find((a) => a.id === target.id);
    if (target.index >= (annotation?.anchors.length ?? 0) || !annotation)
      return null;
    const next: Annotation = {
      ...annotation,
      anchors: replaceAt(annotation.anchors, target.index, fresh),
    };
    return { op: "put_annotation", annotation: next };
  }
  const step = review.tour.find((s) => s.id === target.id);
  if (target.index >= (step?.current.anchors.length ?? 0) || !step) return null;
  const next: TourStep = {
    ...step,
    current: {
      ...step.current,
      anchors: replaceAt(step.current.anchors, target.index, fresh),
    },
  };
  return { op: "put_tour_step", step: next };
}

/** Which immutable document a diff pane shows. */
export type PaneSource = { candidateId: string; side: ReviewSide };
export type DiffPanes = { left: PaneSource; right: PaneSource };
export type PaneId = "left" | "right";

/**
 * Full mode: candidate base on the left, candidate head on the right.
 * Since-last mode: previous candidate head on the left, candidate head on
 * the right. Selections, highlights and reveals map through these sources.
 */
export function diffPanes(
  candidate: Candidate,
  previous: Candidate | null,
  compare: CompareMode,
): DiffPanes {
  const right: PaneSource = { candidateId: candidate.id, side: "head" };
  if (compare === "since_last" && previous) {
    return { left: { candidateId: previous.id, side: "head" }, right };
  }
  return { left: { candidateId: candidate.id, side: "base" }, right };
}

export function paneFor(
  panes: DiffPanes,
  position: { candidate_id: string; side: ReviewSide },
): PaneId | null {
  const matches = (p: PaneSource) =>
    p.candidateId === position.candidate_id && p.side === position.side;
  if (matches(panes.right)) return "right";
  if (matches(panes.left)) return "left";
  return null;
}

export function candidateHasPath(candidate: Candidate, path: string): boolean {
  return candidate.files.some((f) => f.path === path);
}

export function firstAnchorOf(anchors: Anchor[]): Anchor | null {
  return anchors.find((a) => a.status === "current") ?? anchors.at(0) ?? null;
}

export function newRequestKey(): string {
  return crypto.randomUUID();
}

export function shortOid(oid: string): string {
  return oid.slice(0, 8);
}

function historicalId(object: HistoricalObject): string {
  return object.value.id;
}

/** Stored versions of one object that differ from its current state, newest first. */
export function priorRevisions(
  revisions: ObjectRevision[],
  current: HistoricalObject,
): ObjectRevision[] {
  const currentJson = JSON.stringify(current.value);
  return revisions
    .filter(
      (r) =>
        r.object.kind === current.kind &&
        historicalId(r.object) === historicalId(current) &&
        JSON.stringify(r.object.value) !== currentJson,
    )
    .sort((a, b) => b.revision - a.revision);
}
