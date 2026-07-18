import { describe, expect, it } from "vitest";

import type { DaemonEvent } from "../../../services/refact/daemon";
import {
  appendLogLine,
  filterDaemonEvents,
  mergeLogLines,
  timelineFollowAfterScroll,
} from "./activityState";

function daemonEvent(
  seq: number,
  kind: string,
  projectId: string | null,
): DaemonEvent {
  return {
    seq,
    ts_ms: seq,
    kind,
    project_id: projectId,
    payload: { seq },
  };
}

describe("Activity timeline state", () => {
  const events = [
    daemonEvent(1, "worker_started", "p1"),
    daemonEvent(2, "worker_stopped", "p2"),
    daemonEvent(3, "worker_started", "p2"),
  ];

  it("filters the event ring by selected kinds and project", () => {
    expect(
      filterDaemonEvents(events, new Set(["worker_started"]), "p2"),
    ).toEqual([events[2]]);
    expect(filterDaemonEvents(events, new Set(), null)).toEqual(events);
  });

  it("keeps follow pinned at the top and disables it after manual scroll", () => {
    expect(timelineFollowAfterScroll(true, 0)).toBe(true);
    expect(timelineFollowAfterScroll(true, 12)).toBe(false);
    expect(timelineFollowAfterScroll(false, 0)).toBe(false);
  });
});

describe("Activity log tail state", () => {
  it("caps retained lines at the requested tail size", () => {
    const result = ["one", "two", "three", "four"].reduce<string[]>(
      (lines, line) => appendLogLine(lines, line, false, 3),
      [],
    );
    expect(result).toEqual(["two", "three", "four"]);
  });

  it("does not append streamed lines while paused", () => {
    const lines = ["existing"];
    expect(appendLogLine(lines, "ignored", true)).toBe(lines);
  });

  it("flushes buffered paused lines through the cap on resume", () => {
    expect(mergeLogLines(["one", "two"], ["three", "four"], 3)).toEqual([
      "two",
      "three",
      "four",
    ]);
  });
});
