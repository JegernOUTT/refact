import { describe, expect, it } from "vitest";
import { fitPaneTabs, needsCompactEngine } from "./paneTabFit";

describe("workspace tab fitting", () => {
  it("compresses all tabs down to their chrome floors before overflowing", () => {
    expect(fitPaneTabs([70, 70, 70], 210, 30, 2)).toEqual([0, 1, 2]);
    expect(fitPaneTabs([70, 70, 70], 209, 30, 2)).toEqual([0, 2]);
  });

  it("reserves overflow room and keeps the selected tab even in tiny strips", () => {
    expect(fitPaneTabs([70, 90, 70], 170, 30, 1)).toEqual([1]);
    expect(fitPaneTabs([70, 90, 70], 10, 30, 2)).toEqual([2]);
    expect(fitPaneTabs([], 0, 30, -1)).toEqual([]);
  });

  it("uses per-tab floors and stable ordering", () => {
    expect(fitPaneTabs([100, 60, 80], 180, 30, 2)).toEqual([1, 2]);
  });

  it("collapses for natural width pressure before any overflow is needed", () => {
    expect(
      needsCompactEngine([{ left: 0, right: 300, naturalWidth: 301 }], 300),
    ).toBe(true);
    expect(
      needsCompactEngine([{ left: 0, right: 300, naturalWidth: 300 }], 300),
    ).toBe(false);
  });

  it("keeps compact state when only the gained label space makes tabs fit", () => {
    const compact = [{ left: 0, right: 400, naturalWidth: 350 }];
    expect(needsCompactEngine(compact, 300)).toBe(true);
    expect(needsCompactEngine(compact, 360)).toBe(false);
  });

  it("does not assign the last pane's status savings to an earlier pane", () => {
    expect(
      needsCompactEngine(
        [
          { left: 0, right: 200, naturalWidth: 220 },
          { left: 200, right: 600, naturalWidth: 100 },
        ],
        500,
      ),
    ).toBe(true);
    expect(needsCompactEngine([], 500)).toBe(false);
  });
});
