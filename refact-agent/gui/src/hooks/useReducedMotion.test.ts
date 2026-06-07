import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useReducedMotion } from "./useReducedMotion";

const query = "(prefers-reduced-motion: reduce)";

const createMatchMediaMock = () => {
  let matches = false;
  const listeners = new Set<(event: MediaQueryListEvent) => void>();

  const matchMedia = vi.fn((mediaQuery: string): MediaQueryList => {
    return {
      media: mediaQuery,
      matches: mediaQuery === query && matches,
      onchange: null,
      addEventListener: (
        _type: string,
        listener: EventListenerOrEventListenerObject,
      ) => {
        if (typeof listener === "function") {
          listeners.add(listener as (event: MediaQueryListEvent) => void);
        }
      },
      removeEventListener: (
        _type: string,
        listener: EventListenerOrEventListenerObject,
      ) => {
        if (typeof listener === "function") {
          listeners.delete(listener as (event: MediaQueryListEvent) => void);
        }
      },
      addListener: (listener) => {
        if (listener) {
          listeners.add(listener);
        }
      },
      removeListener: (listener) => {
        if (listener) {
          listeners.delete(listener);
        }
      },
      dispatchEvent: () => true,
    };
  });

  const setReducedMotion = (reducedMotion: boolean) => {
    matches = reducedMotion;
    const event = { matches, media: query } as MediaQueryListEvent;
    listeners.forEach((listener) => listener(event));
  };

  return { matchMedia, setReducedMotion };
};

describe("useReducedMotion", () => {
  const originalMatchMedia = window.matchMedia;
  let mock: ReturnType<typeof createMatchMediaMock>;

  beforeEach(() => {
    mock = createMatchMediaMock();
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      value: mock.matchMedia,
    });
  });

  afterEach(() => {
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      value: originalMatchMedia,
    });
  });

  it("returns false by default", () => {
    const { result } = renderHook(() => useReducedMotion());

    expect(result.current).toBe(false);
  });

  it("returns true when reduced motion is preferred", () => {
    mock.setReducedMotion(true);

    const { result } = renderHook(() => useReducedMotion());

    expect(result.current).toBe(true);
  });

  it("updates when the media query changes", () => {
    const { result } = renderHook(() => useReducedMotion());

    expect(result.current).toBe(false);

    act(() => {
      mock.setReducedMotion(true);
    });

    expect(result.current).toBe(true);

    act(() => {
      mock.setReducedMotion(false);
    });

    expect(result.current).toBe(false);
  });
});
