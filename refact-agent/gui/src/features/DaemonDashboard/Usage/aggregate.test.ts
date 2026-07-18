import { describe, expect, it } from "vitest";

import type {
  DayStats,
  ModelStats,
  StatsSummary,
  StatsTotals,
} from "../../StatsDashboard/types";
import {
  aggregateUsage,
  mergeDayStats,
  mergeModelStats,
  type ProjectUsageInput,
} from "./aggregate";

function totals(overrides: Partial<StatsTotals> = {}): StatsTotals {
  return {
    total_calls: 0,
    successful_calls: 0,
    failed_calls: 0,
    total_prompt_tokens: 0,
    total_completion_tokens: 0,
    total_tokens: 0,
    total_cache_read_tokens: 0,
    total_cache_creation_tokens: 0,
    total_cost_usd: null,
    total_duration_ms: 0,
    avg_duration_ms: 0,
    total_conversations: 0,
    total_messages_sent: 0,
    ...overrides,
  };
}

function model(overrides: Partial<ModelStats> = {}): ModelStats {
  return {
    model_id: "anthropic/claude",
    model: "claude",
    provider: "anthropic",
    total_calls: 0,
    successful_calls: 0,
    failed_calls: 0,
    total_prompt_tokens: 0,
    total_completion_tokens: 0,
    total_tokens: 0,
    total_cache_read_tokens: 0,
    total_cache_creation_tokens: 0,
    total_cost_usd: 0,
    total_duration_ms: 0,
    avg_duration_ms: 0,
    ...overrides,
  };
}

function day(overrides: Partial<DayStats> = {}): DayStats {
  return {
    date: "2026-07-01",
    total_calls: 0,
    successful_calls: 0,
    total_prompt_tokens: 0,
    total_completion_tokens: 0,
    total_tokens: 0,
    total_cache_read_tokens: 0,
    total_cache_creation_tokens: 0,
    total_cost_usd: 0,
    total_duration_ms: 0,
    ...overrides,
  };
}

function summary(overrides: Partial<StatsSummary> = {}): StatsSummary {
  return {
    date_range: { from: "2026-07-01", to: "2026-07-18" },
    totals: totals(),
    by_model: [],
    by_provider: [],
    by_day: [],
    by_mode: [],
    top_conversations: [],
    ...overrides,
  };
}

function input(
  projectId: string,
  overrides: Partial<StatsSummary> = {},
): ProjectUsageInput {
  return { projectId, slug: projectId, summary: summary(overrides) };
}

describe("mergeModelStats", () => {
  it("merges colliding model ids and recomputes avg duration", () => {
    const merged = mergeModelStats([
      model({
        total_calls: 4,
        successful_calls: 3,
        failed_calls: 1,
        total_prompt_tokens: 100,
        total_completion_tokens: 50,
        total_tokens: 150,
        total_cache_read_tokens: 10,
        total_cache_creation_tokens: 5,
        total_cost_usd: 1.5,
        total_duration_ms: 4_000,
        avg_duration_ms: 1_000,
      }),
      model({
        total_calls: 6,
        successful_calls: 6,
        failed_calls: 0,
        total_prompt_tokens: 200,
        total_completion_tokens: 100,
        total_tokens: 300,
        total_cache_read_tokens: 20,
        total_cache_creation_tokens: 15,
        total_cost_usd: 2.5,
        total_duration_ms: 6_000,
        avg_duration_ms: 1_000,
      }),
    ]);

    expect(merged).toHaveLength(1);
    expect(merged[0]).toMatchObject({
      model_id: "anthropic/claude",
      total_calls: 10,
      successful_calls: 9,
      failed_calls: 1,
      total_prompt_tokens: 300,
      total_completion_tokens: 150,
      total_tokens: 450,
      total_cache_read_tokens: 30,
      total_cache_creation_tokens: 20,
      total_cost_usd: 4,
      total_duration_ms: 10_000,
      avg_duration_ms: 1_000,
    });
  });

  it("keeps distinct model ids separate sorted by tokens desc", () => {
    const merged = mergeModelStats([
      model({ model_id: "a/small", model: "small", total_tokens: 10 }),
      model({ model_id: "b/big", model: "big", total_tokens: 100 }),
    ]);
    expect(merged.map((entry) => entry.model_id)).toEqual(["b/big", "a/small"]);
  });
});

describe("mergeDayStats", () => {
  it("merges same dates and sorts ascending", () => {
    const merged = mergeDayStats([
      day({ date: "2026-07-02", total_calls: 2, total_tokens: 20 }),
      day({ date: "2026-07-01", total_calls: 1, total_tokens: 10 }),
      day({
        date: "2026-07-02",
        total_calls: 3,
        total_tokens: 30,
        total_cost_usd: 0.5,
      }),
    ]);
    expect(merged.map((entry) => entry.date)).toEqual([
      "2026-07-01",
      "2026-07-02",
    ]);
    expect(merged[1].total_calls).toBe(5);
    expect(merged[1].total_tokens).toBe(50);
    expect(merged[1].total_cost_usd).toBe(0.5);
  });
});

describe("aggregateUsage", () => {
  it("returns zeroed totals for empty inputs", () => {
    const aggregated = aggregateUsage([]);
    expect(aggregated.totals.total_calls).toBe(0);
    expect(aggregated.totals.total_cost_usd).toBeNull();
    expect(aggregated.by_model).toEqual([]);
    expect(aggregated.by_day).toEqual([]);
    expect(aggregated.by_project).toEqual([]);
    expect(aggregated.used_providers).toEqual([]);
  });

  it("keeps aggregate totals equal to the sum of per-project rows", () => {
    const aggregated = aggregateUsage([
      input("alpha", {
        totals: totals({
          total_calls: 10,
          successful_calls: 9,
          failed_calls: 1,
          total_prompt_tokens: 700,
          total_completion_tokens: 300,
          total_tokens: 1_000,
          total_cost_usd: 1.25,
        }),
      }),
      input("beta", {
        totals: totals({
          total_calls: 5,
          successful_calls: 5,
          failed_calls: 0,
          total_prompt_tokens: 300,
          total_completion_tokens: 200,
          total_tokens: 500,
          total_cost_usd: 0.75,
        }),
      }),
    ]);

    const rowCalls = aggregated.by_project.reduce(
      (sum, row) => sum + row.total_calls,
      0,
    );
    const rowTokens = aggregated.by_project.reduce(
      (sum, row) => sum + row.total_tokens,
      0,
    );
    const rowCost = aggregated.by_project.reduce(
      (sum, row) => sum + (row.total_cost_usd ?? 0),
      0,
    );

    expect(aggregated.totals.total_calls).toBe(rowCalls);
    expect(aggregated.totals.total_calls).toBe(15);
    expect(aggregated.totals.successful_calls).toBe(14);
    expect(aggregated.totals.failed_calls).toBe(1);
    expect(aggregated.totals.total_tokens).toBe(rowTokens);
    expect(aggregated.totals.total_tokens).toBe(1_500);
    expect(aggregated.totals.total_prompt_tokens).toBe(1_000);
    expect(aggregated.totals.total_completion_tokens).toBe(500);
    expect(aggregated.totals.total_cost_usd).toBe(rowCost);
    expect(aggregated.totals.total_cost_usd).toBe(2);
  });

  it("keeps cost null when no project reports cost and sums partial costs", () => {
    const noCost = aggregateUsage([
      input("alpha", { totals: totals({ total_calls: 1 }) }),
    ]);
    expect(noCost.totals.total_cost_usd).toBeNull();

    const partial = aggregateUsage([
      input("alpha", { totals: totals({ total_cost_usd: null }) }),
      input("beta", { totals: totals({ total_cost_usd: 3 }) }),
    ]);
    expect(partial.totals.total_cost_usd).toBe(3);
  });

  it("collects used providers from providers and models with calls", () => {
    const aggregated = aggregateUsage([
      input("alpha", {
        by_provider: [
          {
            provider: "anthropic",
            total_calls: 2,
            successful_calls: 2,
            failed_calls: 0,
            total_prompt_tokens: 0,
            total_completion_tokens: 0,
            total_tokens: 0,
            total_cache_read_tokens: 0,
            total_cache_creation_tokens: 0,
            total_cost_usd: 0,
            total_duration_ms: 0,
          },
          {
            provider: "unused",
            total_calls: 0,
            successful_calls: 0,
            failed_calls: 0,
            total_prompt_tokens: 0,
            total_completion_tokens: 0,
            total_tokens: 0,
            total_cache_read_tokens: 0,
            total_cache_creation_tokens: 0,
            total_cost_usd: 0,
            total_duration_ms: 0,
          },
        ],
      }),
      input("beta", {
        by_model: [
          model({
            model_id: "deepseek/chat",
            provider: "deepseek",
            total_calls: 1,
          }),
        ],
      }),
    ]);

    expect(aggregated.used_providers).toEqual(["anthropic", "deepseek"]);
  });

  it("merges by-model and by-day across projects", () => {
    const aggregated = aggregateUsage([
      input("alpha", {
        by_model: [model({ total_calls: 2, total_tokens: 100 })],
        by_day: [day({ date: "2026-07-01", total_tokens: 100 })],
      }),
      input("beta", {
        by_model: [model({ total_calls: 3, total_tokens: 200 })],
        by_day: [day({ date: "2026-07-01", total_tokens: 200 })],
      }),
    ]);

    expect(aggregated.by_model).toHaveLength(1);
    expect(aggregated.by_model[0].total_calls).toBe(5);
    expect(aggregated.by_model[0].total_tokens).toBe(300);
    expect(aggregated.by_day).toHaveLength(1);
    expect(aggregated.by_day[0].total_tokens).toBe(300);
  });
});
