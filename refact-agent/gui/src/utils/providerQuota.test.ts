import { describe, expect, it } from "vitest";

import {
  formatClaudeExtraUsage,
  formatCodexCreditsDetails,
  formatCodexCreditsSummary,
  formatCodexSpendControl,
  formatLimitWindowSeconds,
  formatResetAfterSeconds,
} from "./providerQuota";

describe("provider quota formatting", () => {
  it("formats provider window and reset durations", () => {
    expect(formatLimitWindowSeconds(18_000)).toBe("5 hours");
    expect(formatLimitWindowSeconds(604_800)).toBe("7 days");
    expect(formatLimitWindowSeconds(null)).toBeNull();
    expect(formatResetAfterSeconds(60)).toBe("Resets in 1 minute");
  });

  it("keeps Claude extra usage null values explicit", () => {
    expect(
      formatClaudeExtraUsage({
        is_enabled: false,
        used_credits: null,
        monthly_limit: null,
        utilization: null,
        disabled_reason: "admin_disabled",
      }),
    ).toBe(
      "disabled · admin_disabled · spent not reported · limit not reported",
    );
  });

  it("formats Codex credit and spend-control details", () => {
    expect(
      formatCodexCreditsSummary({
        balance: 0,
        has_credits: false,
        unlimited: false,
      }),
    ).toBe("0 balance · no credits");

    expect(
      formatCodexCreditsDetails({
        balance: 0,
        has_credits: false,
        unlimited: false,
        overage_limit_reached: true,
        approx_cloud_messages: [1, 2.5],
        approx_local_messages: [3, 4],
      }),
    ).toBe("overage reached · cloud approx 1 / 2.5 · local approx 3 / 4");

    expect(
      formatCodexSpendControl({
        reached: false,
        individual_limit: 10.5,
      }),
    ).toBe("reached no · individual limit 10.5");
  });
});
