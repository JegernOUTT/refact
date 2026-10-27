export const GRAPH_COLOR_COUNT = 8;

export type GraphCommitInput = {
  oid: string;
  parents: readonly string[];
};

/** Segment inside one row; lanes are column indexes, colours 0..7. */
export type GraphEdge = {
  from: number;
  to: number;
  color: number;
};

export type GraphRow = {
  oid: string;
  lane: number;
  color: number;
  /** Top half: row top (from) to the commit midline (to). */
  top: GraphEdge[];
  /** Bottom half: commit midline (from) to row bottom (to). */
  bottom: GraphEdge[];
  width: number;
};

type Lane = { oid: string; color: number } | null;

export type GraphState = {
  lanes: readonly Lane[];
  nextColor: number;
};

export type GraphLayout = {
  rows: GraphRow[];
  state: GraphState;
};

export const EMPTY_GRAPH_STATE: GraphState = { lanes: [], nextColor: 0 };

function freeLane(lanes: Lane[], avoid: number): number {
  const index = lanes.findIndex((lane, at) => lane === null && at !== avoid);
  if (index >= 0) return index;
  return Math.max(lanes.length, avoid + 1);
}

function setLane(lanes: Lane[], index: number, lane: Lane) {
  while (lanes.length < index) lanes.push(null);
  lanes[index] = lane;
}

function trimTrailing(lanes: Lane[]): Lane[] {
  let end = lanes.length;
  while (end > 0 && lanes[end - 1] === null) end -= 1;
  return lanes.slice(0, end);
}

function layoutRow(
  commit: GraphCommitInput,
  state: GraphState,
): { row: GraphRow; state: GraphState } {
  const before = state.lanes;
  let nextColor = state.nextColor;
  const takeColor = () => {
    const color = nextColor % GRAPH_COLOR_COUNT;
    nextColor += 1;
    return color;
  };

  const incoming = before.flatMap((lane, index) =>
    lane?.oid === commit.oid ? [index] : [],
  );
  const after: Lane[] = before.map((lane) =>
    lane?.oid === commit.oid ? null : lane,
  );
  const lane = incoming.length > 0 ? incoming[0] : freeLane(after, -1);
  const color = incoming.length > 0 ? before[lane]?.color ?? 0 : takeColor();

  const top: GraphEdge[] = before.flatMap((entry, index) => {
    if (!entry) return [];
    return [
      {
        from: index,
        to: entry.oid === commit.oid ? lane : index,
        color: entry.color,
      },
    ];
  });

  const bottom: GraphEdge[] = [];
  commit.parents.forEach((parent, parentIndex) => {
    const existing = after.findIndex((entry) => entry?.oid === parent);
    if (existing >= 0) {
      bottom.push({
        from: lane,
        to: existing,
        color: parentIndex === 0 ? color : after[existing]?.color ?? color,
      });
      return;
    }
    if (parentIndex === 0 && !after[lane]) {
      setLane(after, lane, { oid: parent, color });
      bottom.push({ from: lane, to: lane, color });
      return;
    }
    const target = freeLane(after, lane);
    const laneColor = takeColor();
    setLane(after, target, { oid: parent, color: laneColor });
    bottom.push({ from: lane, to: target, color: laneColor });
  });

  for (let index = 0; index < after.length; index += 1) {
    const entry = after[index];
    if (index === lane || !entry || before[index] !== entry) continue;
    bottom.push({ from: index, to: index, color: entry.color });
  }

  const lanes = trimTrailing(after);
  const width = Math.max(before.length, lanes.length, lane + 1);
  return {
    row: { oid: commit.oid, lane, color, top, bottom, width },
    state: { lanes, nextColor },
  };
}

/**
 * Assigns lanes to an ordered (topological, newest first) commit list.
 * Pass the previous layout's state to continue with the next page; the
 * result is identical to laying out the concatenated list in one go.
 */
export function layoutCommitGraph(
  commits: readonly GraphCommitInput[],
  state: GraphState = EMPTY_GRAPH_STATE,
): GraphLayout {
  const rows: GraphRow[] = [];
  let current = state;
  for (const commit of commits) {
    const result = layoutRow(commit, current);
    rows.push(result.row);
    current = result.state;
  }
  return { rows, state: current };
}

export function extendCommitGraph(
  layout: GraphLayout,
  commits: readonly GraphCommitInput[],
): GraphLayout {
  const next = layoutCommitGraph(commits, layout.state);
  return { rows: [...layout.rows, ...next.rows], state: next.state };
}
