import { describe, expect, it } from "vitest";
import { defaultGeometry } from "./popoverGeometry";

describe("defaultGeometry", () => {
  it("opens beside the rail without covering its icon on narrow viewports", () => {
    expect(
      defaultGeometry({ width: 480, height: 900 }, { right: 44, bottom: 892 }),
    ).toEqual({ x: 52, y: 292, width: 420, height: 600 });
  });
});
