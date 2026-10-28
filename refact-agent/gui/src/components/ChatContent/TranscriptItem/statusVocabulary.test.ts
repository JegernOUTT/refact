import { describe, expect, it } from "vitest";
import {
  TRANSCRIPT_STATUSES,
  fromAgentStatus,
  fromOutcomeWord,
  fromProcessStatus,
  fromToolTone,
  isQuietStatus,
  transcriptStatusLabel,
} from "./statusVocabulary";

describe("statusVocabulary", () => {
  it("labels every status", () => {
    expect(TRANSCRIPT_STATUSES.map(transcriptStatusLabel)).toEqual([
      "Running",
      "Waiting",
      "Done",
      "Failed",
      "Cancelled",
      "Needs you",
      "Unknown",
    ]);
  });

  it("only done is quiet", () => {
    expect(TRANSCRIPT_STATUSES.filter(isQuietStatus)).toEqual(["done"]);
  });

  it.each([
    ["starting", "running"],
    ["running", "running"],
    ["running_in_background", "running"],
    ["exited", "done"],
    ["failed", "failed"],
    ["killed", "cancelled"],
    ["timed_out", "failed"],
    ["mystery", "unknown"],
    [undefined, "unknown"],
  ] as const)("maps process %s to %s", (input, expected) => {
    expect(fromProcessStatus(input)).toBe(expected);
  });

  it.each([
    ["queued", "waiting"],
    ["running", "running"],
    ["waiting_for_approval", "needs_you"],
    ["completed", "done"],
    ["failed", "failed"],
    ["cancelled", "cancelled"],
    ["interrupted", "cancelled"],
    ["", "unknown"],
  ] as const)("maps agent %s to %s", (input, expected) => {
    expect(fromAgentStatus(input)).toBe(expected);
  });

  it.each([
    ["idle", "done"],
    ["running", "running"],
    ["streaming", "running"],
    ["success", "done"],
    ["error", "failed"],
    ["pending", "waiting"],
    ["succeeded", "done"],
    ["malformed", "failed"],
    ["legacy", "done"],
    ["ready", "done"],
    ["neutral", "done"],
    ["active", "running"],
    ["positive", "done"],
    ["warning", "needs_you"],
    ["danger", "failed"],
    ["accent", "done"],
    ["muted", "done"],
    ["default", "done"],
    ["unknown", "unknown"],
    [null, "unknown"],
  ] as const)("maps tone %s to %s", (input, expected) => {
    expect(fromToolTone(input)).toBe(expected);
  });

  it.each([
    ["Rejected", "failed"],
    ["aborted", "cancelled"],
    ["applied", "done"],
    ["needs_work", "needs_you"],
    ["queued", "waiting"],
    ["whatever", "unknown"],
  ] as const)("maps outcome word %s to %s", (input, expected) => {
    expect(fromOutcomeWord(input)).toBe(expected);
  });
});
