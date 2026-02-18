export interface ModelStats {
  model: string;
  provider: string;
  total_calls: number;
  successful_calls: number;
  failed_calls: number;
  total_prompt_tokens: number;
  total_completion_tokens: number;
  total_tokens: number;
  total_cache_read_tokens: number;
  total_cache_creation_tokens: number;
  total_cost_usd: number | null;
  total_duration_ms: number;
  avg_duration_ms: number;
}

export interface ProviderStats {
  provider: string;
  total_calls: number;
  successful_calls: number;
  failed_calls: number;
  total_prompt_tokens: number;
  total_completion_tokens: number;
  total_tokens: number;
  total_cost_usd: number | null;
  total_duration_ms: number;
}

export interface DayStats {
  date: string;
  total_calls: number;
  successful_calls: number;
  total_tokens: number;
  total_cost_usd: number | null;
  total_duration_ms: number;
}

export interface ModeStats {
  mode: string;
  total_calls: number;
  total_tokens: number;
  total_cost_usd: number | null;
}

export interface ConversationStats {
  chat_id: string;
  title: string;
  model: string;
  mode: string;
  total_calls: number;
  total_tokens: number;
  total_cost_usd: number | null;
  created_at: string;
}

export interface StatsSummary {
  date_range: { from: string; to: string };
  totals: {
    total_calls: number;
    successful_calls: number;
    failed_calls: number;
    total_prompt_tokens: number;
    total_completion_tokens: number;
    total_tokens: number;
    total_cache_read_tokens: number;
    total_cache_creation_tokens: number;
    total_cost_usd: number | null;
    total_duration_ms: number;
    avg_duration_ms: number;
    total_conversations: number;
    total_messages_sent: number;
  };
  by_model: ModelStats[];
  by_provider: ProviderStats[];
  by_day: DayStats[];
  by_mode: ModeStats[];
  top_conversations: ConversationStats[];
}

export interface StatsEventsParams {
  from?: string;
  to?: string;
  limit?: number;
  offset?: number;
  model?: string;
  provider?: string;
}

export interface StatsEvent {
  id: string;
  chat_id: string;
  model: string;
  provider: string;
  mode: string;
  success: boolean;
  prompt_tokens: number;
  completion_tokens: number;
  total_tokens: number;
  cost_usd: number | null;
  duration_ms: number;
  created_at: string;
}

export interface StatsEventsResponse {
  events: StatsEvent[];
  total: number;
  limit: number;
  offset: number;
}

export type DateRangePreset = "7d" | "30d" | "all";

export interface DateRange {
  preset: DateRangePreset;
  from?: string;
  to?: string;
}
