import type { CSSProperties } from "react";

import type { GraphEdge, GraphRow } from "./commitGraphLayout";
import styles from "./GitLog.module.css";

/** viewBox units; CSS sizes one lane to --rf-git-graph-lane and the row to --rf-git-log-row-h. */
const LANE = 12;
const ROW = 26;
const MID = ROW / 2;
const DOT_RADIUS = 3.5;

const laneX = (lane: number) => lane * LANE + LANE / 2;

function edgePath(edge: GraphEdge, y1: number, y2: number): string {
  const x1 = laneX(edge.from);
  const x2 = laneX(edge.to);
  if (x1 === x2) return `M${x1} ${y1}V${y2}`;
  const bend = (y1 + y2) / 2;
  return `M${x1} ${y1}C${x1} ${bend} ${x2} ${bend} ${x2} ${y2}`;
}

export function CommitGraphCell({
  row,
  width,
}: {
  row: GraphRow;
  width: number;
}) {
  return (
    <svg
      aria-hidden="true"
      className={styles.graph}
      focusable="false"
      style={{ "--git-graph-lanes": width } as CSSProperties}
      viewBox={`0 0 ${width * LANE} ${ROW}`}
    >
      {row.top.map((edge, index) => (
        <path
          className={styles.graphEdge}
          d={edgePath(edge, 0, MID)}
          data-color={edge.color}
          key={`t${index}`}
        />
      ))}
      {row.bottom.map((edge, index) => (
        <path
          className={styles.graphEdge}
          d={edgePath(edge, MID, ROW)}
          data-color={edge.color}
          key={`b${index}`}
        />
      ))}
      <circle
        className={styles.graphDot}
        cx={laneX(row.lane)}
        cy={MID}
        data-color={row.color}
        r={DOT_RADIUS}
      />
    </svg>
  );
}
