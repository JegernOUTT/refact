import { describe, expect, it } from "vitest";

import type { ClaudeCodeUsageData } from "../services/refact/providers";

import {
  formatClaudeExtraUsage,
  formatCodexCreditsDetails,
  formatCodexCreditsSummary,
  formatCodexSpendControl,
  formatLimitWindowSeconds,
  formatResetAfterSeconds,
  getClaudeUsageWindowRows,
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

  it("includes model-scoped Claude usage windows", () => {
    expect(
      getClaudeUsageWindowRows({
        five_hour: { percent_used: 12, resets_at: "2026-07-20T00:00:00Z" },
        seven_day: { percent_used: 40 },
        scoped_windows: [
          {
            label: "Fable 5 Max",
            model_id: "claude-fable-5",
            window: {
              percent_used: 68,
              resets_at: "2026-07-21T00:00:00Z",
            },
          },
          {
            label: "Duplicate Fable",
            model_id: "CLAUDE-FABLE-5",
            window: { percent_used: 99 },
          },
          null,
          { label: null, window: { percent_used: 80 } },
          { label: "Malformed window", window: { percent_used: "80" } },
        ],
      } as unknown as ClaudeCodeUsageData),
    ).toEqual([
      {
        key: "five_hour",
        label: "Current session",
        window: { percent_used: 12, resets_at: "2026-07-20T00:00:00Z" },
      },
      {
        key: "seven_day",
        label: "Current week — all models",
        window: { percent_used: 40 },
      },
      {
        key: "scoped:claude-fable-5",
        label: "Current week — Fable 5 Max",
        window: {
          percent_used: 68,
          resets_at: "2026-07-21T00:00:00Z",
        },
      },
    ]);
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
