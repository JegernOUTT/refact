export interface ModelStats {
  model_id: string;
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
  total_cost_usd: number;
  total_cost_coins: number | null;
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
  total_cache_read_tokens: number;
  total_cache_creation_tokens: number;
  total_cost_usd: number;
  total_cost_coins: number | null;
  total_duration_ms: number;
}

export interface DayStats {
  date: string;
  total_calls: number;
  successful_calls: number;
  total_prompt_tokens: number;
  total_completion_tokens: number;
  total_tokens: number;
  total_cache_read_tokens: number;
  total_cache_creation_tokens: number;
  total_cost_usd: number;
  total_cost_coins: number | null;
  total_duration_ms: number;
}

export interface ModeStats {
  mode: string;
  total_calls: number;
  total_tokens: number;
  total_cost_usd: number;
  total_cost_coins: number | null;
}

export interface ConversationStats {
  chat_id: string;
  total_calls: number;
  total_tokens: number;
  total_cost_usd: number;
  total_cost_coins: number | null;
  model_id: string;
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
    total_cost_coins: number | null;
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
  ts_start: string;
  ts_end: string;
  chat_id: string;
  root_chat_id: string | null;
  mode: string;
  task_id: string | null;
  task_role: string | null;
  agent_id: string | null;
  card_id: string | null;
  model_id: string;
  model: string;
  provider: string;
  messages_count: number;
  tools_count: number;
  max_tokens: number;
  temperature: number | null;
  success: boolean;
  error_message: string | null;
  finish_reason: string | null;
  attempt_n: number;
  retry_reason: string | null;
  prompt_tokens: number;
  completion_tokens: number;
  cache_read_tokens: number | null;
  cache_creation_tokens: number | null;
  total_tokens: number;
  cost_usd: number | null;
  cost_coins: number | null;
  duration_ms: number;
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
