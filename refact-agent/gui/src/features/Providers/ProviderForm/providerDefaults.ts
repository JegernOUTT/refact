import type { ProviderDefaults } from "../../../services/refact";

export function normalizeProviderDefaults(
  defaults: ProviderDefaults | undefined,
): ProviderDefaults {
  return {
    ...defaults,
    chat: defaults?.chat ?? {},
    chat_model_2: defaults?.chat_model_2 ?? {},
    task_planner_agent_model: defaults?.task_planner_agent_model ?? {},
    chat_light: defaults?.chat_light ?? {},
    chat_thinking: defaults?.chat_thinking ?? {},
    chat_buddy: defaults?.chat_buddy ?? {},
    completion_model: defaults?.completion_model,
    embedding_model: defaults?.embedding_model,
  };
}
