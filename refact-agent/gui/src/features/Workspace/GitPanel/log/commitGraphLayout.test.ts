import { describe, expect, test } from "vitest";

import {
  extendCommitGraph,
  layoutCommitGraph,
  type GraphCommitInput,
} from "./commitGraphLayout";

const c = (oid: string, ...parents: string[]): GraphCommitInput => ({
  oid,
  parents,
});

const lanes = (commits: GraphCommitInput[]) =>
  layoutCommitGraph(commits).rows.map((row) => row.lane);

describe("layoutCommitGraph", () => {
  test("linear history stays in lane 0 with one colour", () => {
    const { rows, state } = layoutCommitGraph([
      c("c", "b"),
      c("b", "a"),
      c("a"),
    ]);
    expect(rows.map((row) => row.lane)).toEqual([0, 0, 0]);
    expect(new Set(rows.map((row) => row.color)).size).toBe(1);
    expect(rows[0].top).toEqual([]);
    expect(rows[0].bottom).toEqual([{ from: 0, to: 0, color: 0 }]);
    expect(rows[2].bottom).toEqual([]);
    expect(state.lanes).toEqual([]);
  });

  test("branch and merge fork out and join back", () => {
    const { rows } = layoutCommitGraph([
      c("m", "b2", "f2"),
      c("f2", "f1"),
      c("b2", "b1"),
      c("f1", "b1"),
      c("b1", "a"),
      c("a"),
    ]);
    expect(rows.map((row) => row.lane)).toEqual([0, 1, 0, 1, 0, 0]);
    const merge = rows[0];
    expect(merge.bottom).toEqual([
      { from: 0, to: 0, color: merge.color },
      { from: 0, to: 1, color: rows[1].color },
    ]);
    expect(rows[1].color).not.toBe(merge.color);
    expect(rows[3].bottom).toContainEqual({
      from: 1,
      to: 0,
      color: rows[1].color,
    });
    expect(rows[3].width).toBe(2);
    expect(rows[4].top).toEqual([{ from: 0, to: 0, color: merge.color }]);
    expect(rows[4].width).toBe(1);
  });

  test("multiple roots get their own lanes and free them", () => {
    const { rows, state } = layoutCommitGraph([
      c("x2", "x1"),
      c("y1"),
      c("x1"),
    ]);
    expect(rows.map((row) => row.lane)).toEqual([0, 1, 0]);
    expect(rows[1].top).toEqual([{ from: 0, to: 0, color: rows[0].color }]);
    expect(rows[1].bottom).toEqual([{ from: 0, to: 0, color: rows[0].color }]);
    expect(state.lanes).toEqual([]);
  });

  test("reuses the leftmost freed lane", () => {
    expect(
      lanes([
        c("m", "a2", "b2"),
        c("b2", "b1"),
        c("b1"),
        c("n", "a2"),
        c("a2", "a1"),
        c("a1"),
      ]),
    ).toEqual([0, 1, 1, 1, 0, 0]);
  });

  test("paging continuation matches a single layout and keeps open lanes", () => {
    const commits = [
      c("m", "b2", "f2"),
      c("f2", "f1"),
      c("b2", "b1"),
      c("f1", "b1"),
      c("b1", "a"),
      c("a"),
    ];
    const whole = layoutCommitGraph(commits);
    const firstPage = layoutCommitGraph(commits.slice(0, 3));
    expect(firstPage.state.lanes.map((lane) => lane?.oid)).toEqual([
      "b1",
      "f1",
    ]);
    const paged = extendCommitGraph(firstPage, commits.slice(3));
    expect(paged.rows).toEqual(whole.rows);
    expect(paged.state).toEqual(whole.state);
  });
});
