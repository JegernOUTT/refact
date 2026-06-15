import { describe, expect, it } from "vitest";

import { changeFeature, reducer } from "./configSlice";

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
});
