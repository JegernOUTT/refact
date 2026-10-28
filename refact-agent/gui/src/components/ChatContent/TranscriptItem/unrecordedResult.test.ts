import { describe, expect, it } from "vitest";
import {
  BLOCKED_LABEL,
  NEEDS_APPROVAL_LABEL,
  NO_RESULT_LABEL,
  unrecordedResultState,
} from "./unrecordedResult";

describe("unrecordedResultState", () => {
  it.each([
    [{ busy: false }, { status: "unknown", statusLabel: NO_RESULT_LABEL }],
    [{ busy: true }, { status: "running" }],
    [
      { busy: true, pause: "confirmation" as const },
      { status: "needs_you", statusLabel: NEEDS_APPROVAL_LABEL },
    ],
    [
      { busy: false, pause: "unknown" as const },
      { status: "needs_you", statusLabel: NEEDS_APPROVAL_LABEL },
    ],
    [
      { busy: false, pause: "denial" as const },
      { status: "needs_you", statusLabel: BLOCKED_LABEL },
    ],
  ])("%o maps to %o", (input, expected) => {
    expect(unrecordedResultState(input)).toEqual(expected);
  });
});
