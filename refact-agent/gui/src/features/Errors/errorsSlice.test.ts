import { describe, expect, it } from "vitest";

import {
  clearCapsProvidersError,
  clearError,
  errorSlice,
  getCapsProvidersErrorMessage,
  getErrorMessage,
  getRouteErrorMessage,
  setError,
  type ErrorSliceState,
} from "./errorsSlice";

const CAPS_ERROR = {
  message: "caps exploded",
  source: "caps_providers",
} as const;
const ROUTE_ERROR = "deleting integrations.";

function reduce(...actions: Parameters<typeof errorSlice.reducer>[1][]) {
  return actions.reduce<ErrorSliceState>(
    (state, action) => errorSlice.reducer(state, action),
    errorSlice.getInitialState(),
  );
}

describe("errorsSlice error slots", () => {
  it("routes a caps/providers error to the app-level selector only", () => {
    const state = reduce(setError(CAPS_ERROR));

    expect(getCapsProvidersErrorMessage({ error: state })).toBe(
      "caps exploded",
    );
    expect(getRouteErrorMessage({ error: state })).toBeNull();
  });

  it("routes an untagged error to the route-level selector only", () => {
    const state = reduce(setError(ROUTE_ERROR));

    expect(getCapsProvidersErrorMessage({ error: state })).toBeNull();
    expect(getRouteErrorMessage({ error: state })).toBe(ROUTE_ERROR);
  });

  it("stores a route error while a caps/providers error is already stored", () => {
    const state = reduce(setError(CAPS_ERROR), setError(ROUTE_ERROR));

    expect(getCapsProvidersErrorMessage({ error: state })).toBe(
      "caps exploded",
    );
    expect(getRouteErrorMessage({ error: state })).toBe(ROUTE_ERROR);
  });

  it("stores a caps/providers error while a route error is already stored", () => {
    const state = reduce(setError(ROUTE_ERROR), setError(CAPS_ERROR));

    expect(getRouteErrorMessage({ error: state })).toBe(ROUTE_ERROR);
    expect(getCapsProvidersErrorMessage({ error: state })).toBe(
      "caps exploded",
    );
  });

  it("keeps first-wins semantics inside each slot", () => {
    const state = reduce(
      setError(ROUTE_ERROR),
      setError("fetching integrations."),
      setError(CAPS_ERROR),
      setError({ message: "caps exploded again", source: "caps_providers" }),
    );

    expect(getRouteErrorMessage({ error: state })).toBe(ROUTE_ERROR);
    expect(getCapsProvidersErrorMessage({ error: state })).toBe(
      "caps exploded",
    );
  });

  it("clears only the route slot on clearError", () => {
    const state = reduce(
      setError(CAPS_ERROR),
      setError(ROUTE_ERROR),
      clearError(),
    );

    expect(getRouteErrorMessage({ error: state })).toBeNull();
    expect(getCapsProvidersErrorMessage({ error: state })).toBe(
      "caps exploded",
    );
  });

  it("clears only the caps slot on clearCapsProvidersError", () => {
    const state = reduce(
      setError(CAPS_ERROR),
      setError(ROUTE_ERROR),
      clearCapsProvidersError(),
    );

    expect(getCapsProvidersErrorMessage({ error: state })).toBeNull();
    expect(getRouteErrorMessage({ error: state })).toBe(ROUTE_ERROR);
  });

  it("prefers the route message in the legacy getErrorMessage selector", () => {
    const capsOnly = reduce(setError(CAPS_ERROR));
    expect(getErrorMessage({ error: capsOnly })).toBe("caps exploded");

    const both = reduce(setError(CAPS_ERROR), setError(ROUTE_ERROR));
    expect(getErrorMessage({ error: both })).toBe(ROUTE_ERROR);

    const routeCleared = reduce(
      setError(CAPS_ERROR),
      setError(ROUTE_ERROR),
      clearError(),
    );
    expect(getErrorMessage({ error: routeCleared })).toBe("caps exploded");
  });
});
