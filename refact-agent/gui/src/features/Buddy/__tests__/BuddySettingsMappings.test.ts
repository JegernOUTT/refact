import { describe, expect, it } from "vitest";
import { buildBuddyNeedRows } from "../sections/buddyNeedRows";
import {
  SPEECH_INTENT_KEYS,
  formatChatReactionStatus,
  humorIntentBlocked,
  isIntentAllowed,
  nextMutedIntents,
  speechReasonLabel,
} from "../BuddySettingsPanelLabels";
import type { BuddyPetState, BuddySettings } from "../types";

function pet(needs: Partial<BuddyPetState["needs"]>): BuddyPetState {
  return {
    needs: {
      hunger: 50,
      energy: 50,
      hygiene: 50,
      boredom: 50,
      affection: 50,
      ...needs,
    },
  } as BuddyPetState;
}

describe("buddyNeedRows", () => {
  it("fills every bar proportionally to the number it shows", () => {
    const rows = buildBuddyNeedRows(
      pet({ hunger: 0, energy: 37, boredom: 100, affection: 92 }),
    );
    for (const row of rows) expect(row.fill).toBe(row.value);
    expect(rows.find((row) => row.key === "boredom")).toMatchObject({
      value: 100,
      fill: 100,
      tone: "danger",
    });
  });

  it("tones high boredom as bad and low boredom as good", () => {
    const tones = (boredom: number) =>
      buildBuddyNeedRows(pet({ boredom })).find((row) => row.key === "boredom")
        ?.tone;
    expect(tones(10)).toBe("success");
    expect(tones(70)).toBe("warning");
    expect(tones(90)).toBe("danger");
  });

  it("tones low hunger, energy, hygiene and affection as bad", () => {
    const rows = buildBuddyNeedRows(
      pet({ hunger: 5, energy: 30, hygiene: 90, affection: 10 }),
    );
    const tone = (key: string) => rows.find((row) => row.key === key)?.tone;
    expect(tone("hunger")).toBe("danger");
    expect(tone("energy")).toBe("warning");
    expect(tone("hygiene")).toBe("success");
    expect(tone("affection")).toBe("danger");
  });

  it("clamps out-of-range and missing values", () => {
    const rows = buildBuddyNeedRows(pet({ energy: 140, hygiene: -5 }));
    expect(rows.find((row) => row.key === "energy")?.fill).toBe(100);
    expect(rows.find((row) => row.key === "hygiene")?.fill).toBe(0);
    expect(buildBuddyNeedRows(undefined).every((row) => row.value === 0)).toBe(
      true,
    );
  });
});

describe("speech intent allowed mapping", () => {
  it("shows an intent as allowed exactly when it is not muted", () => {
    expect(isIntentAllowed([], "humor")).toBe(true);
    expect(isIntentAllowed(["humor"], "humor")).toBe(false);
    expect(isIntentAllowed(["humor"], "win")).toBe(true);
  });

  it("turning a topic off appends it to muted_intents", () => {
    expect(nextMutedIntents(["win"], "humor", false)).toEqual(["win", "humor"]);
    expect(nextMutedIntents(["humor"], "humor", false)).toEqual(["humor"]);
  });

  it("turning a topic on removes it from muted_intents", () => {
    expect(nextMutedIntents(["win", "humor"], "humor", true)).toEqual(["win"]);
    expect(nextMutedIntents([], "humor", true)).toEqual([]);
  });

  it("round-trips every intent without touching the others", () => {
    const muted = ["insight", "tour"];
    for (const key of SPEECH_INTENT_KEYS) {
      const off = nextMutedIntents(muted, key, false);
      expect(isIntentAllowed(off, key)).toBe(false);
      const on = nextMutedIntents(off, key, true);
      expect(isIntentAllowed(on, key)).toBe(true);
      for (const other of muted.filter((item) => item !== key))
        expect(on).toContain(other);
    }
  });

  it("flags humor as blocked when the humor setting is off", () => {
    const base = { humor_enabled: true, humor_level: "light" } as BuddySettings;
    expect(humorIntentBlocked(base)).toBe(false);
    expect(humorIntentBlocked({ ...base, humor_enabled: false })).toBe(true);
    expect(humorIntentBlocked({ ...base, humor_level: "off" })).toBe(true);
  });
});

describe("speech reason labels", () => {
  it("maps engine reason codes to one consistent label set", () => {
    expect(speechReasonLabel("allowed", true)).toBe("Spoke on its own");
    expect(speechReasonLabel("user_initiated", true)).toBe("You asked");
    expect(speechReasonLabel("quiet_hours", false)).toBe(
      "Held back: quiet hours",
    );
    expect(speechReasonLabel("brand_new_code", false)).toBe("Held back");
  });

  it("summarises chat reaction activity without raw codes", () => {
    expect(formatChatReactionStatus(null)).toBe(
      "No chat reaction activity yet.",
    );
    expect(
      formatChatReactionStatus({
        recent_attempts: [],
        counts_by_result: {},
        last_skip_reason: "cooldown_active",
      }),
    ).toBe("Last skipped: cooldown active.");
  });
});
