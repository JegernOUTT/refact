import { describe, expect, it } from "vitest";
import { parseWorkPage, workSequence, workStates } from "./workTypes";

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
