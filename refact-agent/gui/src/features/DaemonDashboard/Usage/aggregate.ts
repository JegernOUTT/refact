import type {
  DayStats,
  ModelStats,
  StatsSummary,
} from "../../StatsDashboard/types";

export type ProjectUsageInput = {
  projectId: string;
  slug: string;
  summary: StatsSummary;
};

export type ProjectUsageRow = {
  projectId: string;
  slug: string;
  total_calls: number;
  successful_calls: number;
  total_tokens: number;
  total_cost_usd: number | null;
};

export type AggregatedTotals = {
  total_calls: number;
  successful_calls: number;
  failed_calls: number;
  total_prompt_tokens: number;
  total_completion_tokens: number;
  total_tokens: number;
  total_cost_usd: number | null;
};

export type AggregatedUsage = {
  totals: AggregatedTotals;
  by_model: ModelStats[];
  by_day: DayStats[];
  by_project: ProjectUsageRow[];
  used_providers: string[];
};

function addNullableCost(
  current: number | null,
  next: number | null,
): number | null {
  if (next === null) return current;
  return (current ?? 0) + next;
}

export function mergeModelStats(models: ModelStats[]): ModelStats[] {
  const byId = new Map<string, ModelStats>();
  for (const model of models) {
    const existing = byId.get(model.model_id);
    if (!existing) {
      byId.set(model.model_id, { ...model });
      continue;
    }
    existing.total_calls += model.total_calls;
    existing.successful_calls += model.successful_calls;
    existing.failed_calls += model.failed_calls;
    existing.total_prompt_tokens += model.total_prompt_tokens;
    existing.total_completion_tokens += model.total_completion_tokens;
    existing.total_tokens += model.total_tokens;
    existing.total_cache_read_tokens += model.total_cache_read_tokens;
    existing.total_cache_creation_tokens += model.total_cache_creation_tokens;
    existing.total_cost_usd += model.total_cost_usd;
    existing.total_duration_ms += model.total_duration_ms;
  }
  const merged = [...byId.values()].map((model) => ({
    ...model,
    avg_duration_ms:
      model.total_calls > 0
        ? Math.round(model.total_duration_ms / model.total_calls)
        : 0,
  }));
  return merged.sort((left, right) => right.total_tokens - left.total_tokens);
}

export function mergeDayStats(days: DayStats[]): DayStats[] {
  const byDate = new Map<string, DayStats>();
  for (const day of days) {
    const existing = byDate.get(day.date);
    if (!existing) {
      byDate.set(day.date, { ...day });
      continue;
    }
    existing.total_calls += day.total_calls;
    existing.successful_calls += day.successful_calls;
    existing.total_prompt_tokens += day.total_prompt_tokens;
    existing.total_completion_tokens += day.total_completion_tokens;
    existing.total_tokens += day.total_tokens;
    existing.total_cache_read_tokens += day.total_cache_read_tokens;
    existing.total_cache_creation_tokens += day.total_cache_creation_tokens;
    existing.total_cost_usd += day.total_cost_usd;
    existing.total_duration_ms += day.total_duration_ms;
  }
  return [...byDate.values()].sort((left, right) =>
    left.date.localeCompare(right.date),
  );
}

function usedProviders(inputs: ProjectUsageInput[]): string[] {
  const providers = new Set<string>();
  for (const input of inputs) {
    for (const provider of input.summary.by_provider) {
      if (provider.total_calls > 0) providers.add(provider.provider);
    }
    for (const model of input.summary.by_model) {
      if (model.total_calls > 0) providers.add(model.provider);
    }
  }
  return [...providers].sort((left, right) => left.localeCompare(right));
}

export function aggregateUsage(inputs: ProjectUsageInput[]): AggregatedUsage {
  const totals: AggregatedTotals = {
    total_calls: 0,
    successful_calls: 0,
    failed_calls: 0,
    total_prompt_tokens: 0,
    total_completion_tokens: 0,
    total_tokens: 0,
    total_cost_usd: null,
  };
  const byProject: ProjectUsageRow[] = [];

  for (const input of inputs) {
    const source = input.summary.totals;
    totals.total_calls += source.total_calls;
    totals.successful_calls += source.successful_calls;
    totals.failed_calls += source.failed_calls;
    totals.total_prompt_tokens += source.total_prompt_tokens;
    totals.total_completion_tokens += source.total_completion_tokens;
    totals.total_tokens += source.total_tokens;
    totals.total_cost_usd = addNullableCost(
      totals.total_cost_usd,
      source.total_cost_usd,
    );
    byProject.push({
      projectId: input.projectId,
      slug: input.slug,
      total_calls: source.total_calls,
      successful_calls: source.successful_calls,
      total_tokens: source.total_tokens,
      total_cost_usd: source.total_cost_usd,
    });
  }

  return {
    totals,
    by_model: mergeModelStats(
      inputs.flatMap((input) => input.summary.by_model),
    ),
    by_day: mergeDayStats(inputs.flatMap((input) => input.summary.by_day)),
    by_project: byProject,
    used_providers: usedProviders(inputs),
  };
}
