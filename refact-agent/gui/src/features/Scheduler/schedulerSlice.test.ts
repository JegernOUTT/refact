import { describe, expect, it } from "vitest";
import {
  cronFireReceived,
  schedulerSlice,
  setSchedulerScope,
} from "./schedulerSlice";

describe("schedulerSlice", () => {
  it("updates scope", () => {
    const state = schedulerSlice.reducer(
      undefined,
      setSchedulerScope("durable"),
    );

    expect(state).toEqual({
      scope: "durable",
      lastCronFireAt: null,
    });
  });

  it("records cron fire time", () => {
    const state = schedulerSlice.reducer(undefined, cronFireReceived(1234));

    expect(state).toEqual({
      scope: "all",
      lastCronFireAt: 1234,
    });
  });
});
