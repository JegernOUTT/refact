export function hasReasoningSupport(model: {
  reasoning_effort_options?: string[] | null;
  supports_thinking_budget?: boolean;
  supports_adaptive_thinking_budget?: boolean;
}): boolean {
  return (
    !!model.reasoning_effort_options?.length ||
    !!model.supports_thinking_budget ||
    !!model.supports_adaptive_thinking_budget
  );
}
