import { describe, expect, it } from "vitest";

import {
  clearError,
  errorSlice,
  getCapsProvidersErrorMessage,
  getErrorSource,
  getRouteErrorMessage,
  setError,
  type ErrorSliceState,
} from "./errorsSlice";

function reduce(...actions: Parameters<typeof errorSlice.reducer>[1][]) {
  return actions.reduce<ErrorSliceState>(
    (state, action) => errorSlice.reducer(state, action),
    errorSlice.getInitialState(),
  );
}

describe("errorsSlice error source", () => {
  it("routes a caps/providers error to the app-level selector only", () => {
    const state = reduce(
      setError({ message: "caps exploded", source: "caps_providers" }),
    );

    expect(getErrorSource({ error: state })).toBe("caps_providers");
    expect(getCapsProvidersErrorMessage({ error: state })).toBe(
      "caps exploded",
    );
    expect(getRouteErrorMessage({ error: state })).toBeNull();
  });

  it("routes an untagged error to the route-level selector only", () => {
    const state = reduce(setError("deleting integrations."));

    expect(getErrorSource({ error: state })).toBeUndefined();
    expect(getCapsProvidersErrorMessage({ error: state })).toBeNull();
    expect(getRouteErrorMessage({ error: state })).toBe(
      "deleting integrations.",
    );
  });

  it("drops the source when the error is cleared", () => {
    const state = reduce(
      setError({ message: "caps exploded", source: "caps_providers" }),
      clearError(),
    );

    expect(getErrorSource({ error: state })).toBeUndefined();
    expect(getCapsProvidersErrorMessage({ error: state })).toBeNull();
    expect(getRouteErrorMessage({ error: state })).toBeNull();
  });
});
