import { describe, expect, it } from "vitest";

import { changeFeature, reducer, updateConfig } from "./configSlice";

describe("configSlice", () => {
  it("updates a feature flag without dropping existing flags", () => {
    const state = reducer(
      undefined,
      changeFeature({ feature: "images", value: false }),
    );

    expect(state.features?.images).toBe(false);
    expect(state.features?.statistics).toBe(true);
    expect(state.features?.vecdb).toBe(true);
    expect(state.features?.ast).toBe(true);
  });

  it("persists plugin backend readiness fields", () => {
    const startingState = reducer(
      undefined,
      updateConfig({ backendReady: false, connectionStatus: "starting" }),
    );

    expect(startingState.backendReady).toBe(false);
    expect(startingState.connectionStatus).toBe("starting");

    const readyState = reducer(
      startingState,
      updateConfig({ backendReady: true, connectionStatus: "ready" }),
    );

    expect(readyState.backendReady).toBe(true);
    expect(readyState.connectionStatus).toBe("ready");
  });
});
