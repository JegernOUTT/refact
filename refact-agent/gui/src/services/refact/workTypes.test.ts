import { describe, expect, it } from "vitest";
import {
  parseWorkFileOverlaps,
  parseWorkPage,
  withheldRowCount,
  workSequence,
  workStates,
} from "./workTypes";

import { workFixture } from "./workFixtures";

describe("Work contract", () => {
  it("accepts both runtime states and unknown measured usage", () => {
    for (const state of workStates) {
      const page = structuredClone(workFixture);
      page.rows[0].state = state;
      expect(parseWorkPage(page)).not.toBeNull();
    }
  });
  it("rejects invalid nested payloads and false page counts", () => {
    for (const change of [
      { seq: -1 },
      { returned: 2 },
      { rows: [{}] },
      { roots: [{}] },
      { rows: [{ ...workFixture.rows[0], observed_reads: 7 }] },
    ]) {
      expect(parseWorkPage({ ...workFixture, ...change })).toBeNull();
    }
  });
  it("requires a fresh snapshot after gaps, restart or project change", () => {
    expect(workSequence(null, workFixture)).toBe("accept");
    expect(workSequence(workFixture, workFixture)).toBe("duplicate");
    expect(workSequence(workFixture, { ...workFixture, seq: 2 })).toBe(
      "accept",
    );
    expect(workSequence(workFixture, { ...workFixture, seq: 3 })).toBe("reset");
    expect(
      workSequence(workFixture, { ...workFixture, snapshot_id: "restart" }),
    ).toBe("reset");
    expect(
      workSequence(workFixture, { ...workFixture, project: "other" }),
    ).toBe("reset");
  });
});

describe("Work file overlap", () => {
  const page = {
    ...workFixture,
    rows: [
      workFixture.rows[0],
      { ...workFixture.rows[0], key: { runtime: "trajectory", id: "peer" } },
    ],
    total: 2,
    returned: 2,
  };
  const selector = { kind: "files", paths: ["src/lib.rs"] };

  it("classifies a reported key as advisory and every other row as physical", () => {
    const overlaps = parseWorkFileOverlaps(
      { selector, integration_overlaps: ["trajectory:peer"] },
      page,
    );
    expect(overlaps?.paths).toEqual(["src/lib.rs"]);
    expect(overlaps?.overlaps.map((entry) => entry.overlap)).toEqual([
      "physical",
      "advisory",
    ]);
  });

  it("treats an omitted overlap list as physical matches only", () => {
    const overlaps = parseWorkFileOverlaps({ selector }, page);
    expect(overlaps?.overlaps.map((entry) => entry.overlap)).toEqual([
      "physical",
      "physical",
    ]);
  });

  it("refuses any selector that is not a files inspection", () => {
    for (const value of [
      null,
      {},
      { selector: { kind: "work", work_id: "chat" } },
      { selector: { kind: "files" } },
      { selector, integration_overlaps: "trajectory:peer" },
    ]) {
      expect(parseWorkFileOverlaps(value, page)).toBeNull();
    }
  });
});

describe("Withheld row count", () => {
  it("reports a count only when the payload carries a positive one", () => {
    expect(withheldRowCount(workFixture)).toBeNull();
    expect(withheldRowCount({ ...workFixture, withheld_rows: 0 })).toBeNull();
    expect(withheldRowCount({ ...workFixture, withheld_rows: 2 })).toBe(2);
  });
  it("rejects a page carrying a malformed count", () => {
    expect(parseWorkPage({ ...workFixture, withheld_rows: -1 })).toBeNull();
    expect(parseWorkPage({ ...workFixture, withheld_rows: "2" })).toBeNull();
    expect(parseWorkPage({ ...workFixture, withheld_rows: 4 })).not.toBeNull();
  });
});
