import { renderHook } from "@testing-library/react";
import { act } from "react-dom/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { useIsDocumentVisible } from "../hooks/useIsDocumentVisible";

describe("useIsDocumentVisible", () => {
  const originalVisibilityState = Object.getOwnPropertyDescriptor(
    document,
    "visibilityState",
  );
  let visibilityState: DocumentVisibilityState;

  beforeEach(() => {
    visibilityState = "visible";
    Object.defineProperty(document, "visibilityState", {
      configurable: true,
      get: () => visibilityState,
    });
  });

  afterEach(() => {
    vi.restoreAllMocks();
    if (originalVisibilityState) {
      Object.defineProperty(
        document,
        "visibilityState",
        originalVisibilityState,
      );
    } else {
      Reflect.deleteProperty(document, "visibilityState");
    }
  });

  it("tracks document visibility changes", () => {
    visibilityState = "hidden";
    const { result } = renderHook(() => useIsDocumentVisible());

    expect(result.current).toBe(false);

    act(() => {
      visibilityState = "visible";
      document.dispatchEvent(new Event("visibilitychange"));
    });

    expect(result.current).toBe(true);

    act(() => {
      visibilityState = "hidden";
      document.dispatchEvent(new Event("visibilitychange"));
    });

    expect(result.current).toBe(false);
  });

  it("removes the visibility listener on unmount", () => {
    const removeEventListener = vi.spyOn(document, "removeEventListener");
    const { unmount } = renderHook(() => useIsDocumentVisible());

    unmount();

    expect(removeEventListener).toHaveBeenCalledWith(
      "visibilitychange",
      expect.any(Function),
    );
  });
});
