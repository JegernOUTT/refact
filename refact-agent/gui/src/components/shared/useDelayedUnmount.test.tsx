import { renderHook } from "@testing-library/react";
import { act } from "react-dom/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useDelayedUnmount } from "./useDelayedUnmount";

const reducedMotionQuery = "(prefers-reduced-motion: reduce)";

type MatchMediaMock = ReturnType<typeof createMatchMediaMock>;

function createMatchMediaMock() {
  let matches = false;
  const listeners = new Set<(event: MediaQueryListEvent) => void>();

  const matchMedia = vi.fn((mediaQuery: string): MediaQueryList => {
    return {
      media: mediaQuery,
      matches: mediaQuery === reducedMotionQuery && matches,
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
      addListener: (listener: (event: MediaQueryListEvent) => void) => {
        listeners.add(listener);
      },
      removeListener: (listener: (event: MediaQueryListEvent) => void) => {
        listeners.delete(listener);
      },
      dispatchEvent: () => true,
    };
  });

  return {
    matchMedia,
    setReducedMotion: (enabled: boolean) => {
      matches = enabled;
      const event = {
        matches,
        media: reducedMotionQuery,
      } as MediaQueryListEvent;
      listeners.forEach((listener) => listener(event));
    },
  };
}

describe("useDelayedUnmount", () => {
  const originalMatchMedia = window.matchMedia;
  let media: MatchMediaMock;

  beforeEach(() => {
    media = createMatchMediaMock();
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      value: media.matchMedia,
    });
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      value: originalMatchMedia,
    });
  });

  it("resolves open content to rendered and visually open without waiting for raf", () => {
    const { result } = renderHook(() => useDelayedUnmount(true));

    expect(result.current).toEqual({
      shouldRender: true,
      isAnimatingOpen: true,
    });
  });

  it("delays unmount while closing", () => {
    vi.useFakeTimers();
    const { result, rerender } = renderHook(
      ({ isOpen }) => useDelayedUnmount(isOpen),
      { initialProps: { isOpen: true } },
    );

    rerender({ isOpen: false });

    expect(result.current.shouldRender).toBe(true);
    expect(result.current.isAnimatingOpen).toBe(false);

    act(() => {
      vi.advanceTimersByTime(149);
    });
    expect(result.current.shouldRender).toBe(true);

    act(() => {
      vi.advanceTimersByTime(1);
    });
    expect(result.current.shouldRender).toBe(false);
  });

  it("applies the open visual immediately when reopening before the close delay elapses", () => {
    vi.useFakeTimers();
    const { result, rerender } = renderHook(
      ({ isOpen }) => useDelayedUnmount(isOpen),
      { initialProps: { isOpen: true } },
    );

    rerender({ isOpen: false });
    act(() => {
      vi.advanceTimersByTime(149);
    });

    expect(result.current).toEqual({
      shouldRender: true,
      isAnimatingOpen: false,
    });

    rerender({ isOpen: true });

    expect(result.current).toEqual({
      shouldRender: true,
      isAnimatingOpen: true,
    });
  });

  it("reopens after unmount with the open visual applied", () => {
    vi.useFakeTimers();
    const { result, rerender } = renderHook(
      ({ isOpen }) => useDelayedUnmount(isOpen),
      { initialProps: { isOpen: true } },
    );

    rerender({ isOpen: false });
    act(() => {
      vi.advanceTimersByTime(150);
    });
    expect(result.current.shouldRender).toBe(false);

    rerender({ isOpen: true });

    expect(result.current).toEqual({
      shouldRender: true,
      isAnimatingOpen: true,
    });
  });

  it("uses instant state changes when reduced motion is enabled", () => {
    vi.useFakeTimers();
    media.setReducedMotion(true);
    const { result, rerender } = renderHook(
      ({ isOpen }) => useDelayedUnmount(isOpen),
      { initialProps: { isOpen: true } },
    );

    rerender({ isOpen: false });

    expect(result.current).toEqual({
      shouldRender: false,
      isAnimatingOpen: false,
    });

    act(() => {
      vi.advanceTimersByTime(150);
    });
    expect(result.current.shouldRender).toBe(false);
  });

  it("does not strand content mounted but visually closed after rapid open close open", () => {
    vi.useFakeTimers();
    const { result, rerender } = renderHook(
      ({ isOpen }) => useDelayedUnmount(isOpen),
      { initialProps: { isOpen: false } },
    );

    rerender({ isOpen: true });
    rerender({ isOpen: false });
    rerender({ isOpen: true });

    expect(result.current.shouldRender).toBe(true);

    act(() => {
      vi.runOnlyPendingTimers();
    });

    expect(result.current.shouldRender).toBe(true);
    expect(result.current.isAnimatingOpen).toBe(true);
  });

  it("falls back from a cancelled raf so mounted content still opens", () => {
    vi.useFakeTimers();
    const requestAnimationFrame = vi.fn(() => 1);
    const cancelAnimationFrame = vi.fn();
    vi.stubGlobal("requestAnimationFrame", requestAnimationFrame);
    vi.stubGlobal("cancelAnimationFrame", cancelAnimationFrame);
    const { result, rerender } = renderHook(
      ({ isOpen }) => useDelayedUnmount(isOpen),
      { initialProps: { isOpen: true } },
    );

    rerender({ isOpen: false });
    rerender({ isOpen: true });

    act(() => {
      vi.runOnlyPendingTimers();
    });

    expect(result.current.shouldRender).toBe(true);
    expect(result.current.isAnimatingOpen).toBe(true);
  });
});
