import { renderHook } from "@testing-library/react";
import { act } from "react-dom/test-utils";
import { describe, expect, test, vi } from "vitest";
import {
  MAX_HISTORY_CHARS,
  MAX_HISTORY_ENTRIES,
  useUndoRedo,
} from "./useUndoRedo";

describe("useUndoRedo", () => {
  test("bounds incremental typing history while retaining an undo point", () => {
    vi.useFakeTimers();
    const { result } = renderHook(() => useUndoRedo(""));

    act(() => {
      for (let length = 1; length <= 5_000; length++) {
        result.current.setState("x".repeat(length));
        vi.advanceTimersByTime(1);
      }
    });

    expect(result.current.pastStates.length).toBeLessThanOrEqual(
      MAX_HISTORY_ENTRIES,
    );
    expect(
      result.current.pastStates.reduce((sum, value) => sum + value.length, 0),
    ).toBeLessThanOrEqual(MAX_HISTORY_CHARS);
    expect(result.current.pastStates.length).toBeLessThan(1_000);

    act(() => result.current.undo());
    expect(result.current.state.length).toBeGreaterThan(0);
    expect(result.current.state.length).toBeLessThan(5_000);
    vi.useRealTimers();
  });

  test("drops the oldest entries when retained characters exceed the cap", () => {
    const { result } = renderHook(() => useUndoRedo(""));

    act(() => {
      for (let index = 1; index <= 2_000; index++) {
        result.current.setState(`${index}:`.padEnd(20_000, "x"));
      }
    });

    expect(result.current.pastStates.length).toBeLessThanOrEqual(
      MAX_HISTORY_ENTRIES,
    );
    expect(
      result.current.pastStates.reduce((sum, value) => sum + value.length, 0),
    ).toBeLessThanOrEqual(MAX_HISTORY_CHARS);
    expect(result.current.pastStates.at(0)).not.toContain("1:");
  });

  test("retains one undo entry when a single snapshot exceeds the char cap", () => {
    const { result } = renderHook(() => useUndoRedo(""));

    const huge = "y".repeat(MAX_HISTORY_CHARS + 10);

    act(() => {
      result.current.setState(huge);
      result.current.setState(`${huge}zzzzzzzzzz`);
    });

    expect(result.current.pastStates.length).toBe(1);
    expect(result.current.isUndoPossible).toBe(true);

    act(() => result.current.undo());
    expect(result.current.state).toBe(huge);
  });

  test("keeps a mid-string insertion as its own undo step", () => {
    vi.useFakeTimers();
    const { result } = renderHook(() => useUndoRedo(""));

    const base = "a".repeat(2_000);

    act(() => {
      result.current.setState(base);
      vi.advanceTimersByTime(1);
      result.current.setState(
        `${base.slice(0, 1_000)}@abc${base.slice(1_000)}`,
      );
      vi.advanceTimersByTime(1);
    });

    expect(result.current.pastStates.at(-1)).toBe(base);

    act(() => result.current.undo());
    expect(result.current.state).toBe(base);
    vi.useRealTimers();
  });

  test("coalesces end appends and end deletions only", () => {
    vi.useFakeTimers();
    const { result } = renderHook(() => useUndoRedo(""));

    const base = "b".repeat(2_000);

    act(() => {
      result.current.setState(base);
      vi.advanceTimersByTime(1);
    });

    const pastAfterBase = result.current.pastStates.length;

    act(() => {
      result.current.setState(`${base}c`);
      vi.advanceTimersByTime(1);
      result.current.setState(`${base}cd`);
      vi.advanceTimersByTime(1);
      result.current.setState(`${base}c`);
      vi.advanceTimersByTime(1);
    });

    expect(result.current.pastStates.length).toBe(pastAfterBase);
    vi.useRealTimers();
  });
});
