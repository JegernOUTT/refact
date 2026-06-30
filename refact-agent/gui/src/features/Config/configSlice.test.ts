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

  it("updates plugin backend readiness fields by property presence", () => {
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

    const omittedState = reducer(readyState, updateConfig({ lspPort: 8002 }));

    expect(omittedState.backendReady).toBe(true);
    expect(omittedState.connectionStatus).toBe("ready");
  });

  it("clears plugin backend readiness fields when present with undefined", () => {
    const readyState = reducer(
      undefined,
      updateConfig({ backendReady: true, connectionStatus: "ready" }),
    );

    const clearedState = reducer(
      readyState,
      updateConfig({ backendReady: undefined, connectionStatus: undefined }),
    );

    expect(clearedState.backendReady).toBeUndefined();
    expect(clearedState.connectionStatus).toBeUndefined();
  });

  it("updates plugin backend URLs by property presence", () => {
    const readyState = reducer(
      undefined,
      updateConfig({
        lspUrl: "http://127.0.0.1:8488/p/project",
        browserUrl: "http://workstation.local:8488/p/project",
      }),
    );

    expect(readyState.lspUrl).toBe("http://127.0.0.1:8488/p/project");
    expect(readyState.browserUrl).toBe(
      "http://workstation.local:8488/p/project",
    );

    const partialState = reducer(
      readyState,
      updateConfig({ themeProps: { appearance: "light" } }),
    );

    expect(partialState.lspUrl).toBe("http://127.0.0.1:8488/p/project");
    expect(partialState.browserUrl).toBe(
      "http://workstation.local:8488/p/project",
    );

    const clearedState = reducer(
      partialState,
      updateConfig({ lspUrl: null, browserUrl: null }),
    );

    expect(clearedState.lspUrl).toBeUndefined();
    expect(clearedState.browserUrl).toBeUndefined();
  });
});
