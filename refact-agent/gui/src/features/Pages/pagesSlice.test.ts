import { describe, expect, it } from "vitest";
import { openScheduler, pagesSlice, push } from "./pagesSlice";

describe("pagesSlice", () => {
  it("opens scheduler with task deep-link payload", () => {
    const state = pagesSlice.reducer(
      undefined,
      openScheduler({ taskId: "task-1" }),
    );

    expect(state.at(-1)).toEqual({ name: "scheduler", taskId: "task-1" });
  });

  it("opens scheduler without task deep-link payload", () => {
    const state = pagesSlice.reducer(undefined, openScheduler(undefined));

    expect(state.at(-1)).toEqual({ name: "scheduler" });
  });

  it("registers code intelligence page entries", () => {
    const state = pagesSlice.reducer(undefined, push({ name: "code intel" }));

    expect(state.at(-1)).toEqual({ name: "code intel" });
  });
});
