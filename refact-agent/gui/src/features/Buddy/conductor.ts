import type { ConductorGoal } from "./types";

export function conductorGoalCounts(goal: ConductorGoal): {
  planners: number;
  cards: number;
  agents: number;
} {
  return {
    planners: goal.ledger.planner_task_id ? 1 : 0,
    cards: goal.ledger.task_ids.length,
    agents: goal.ledger.chat_ids.length,
  };
}
