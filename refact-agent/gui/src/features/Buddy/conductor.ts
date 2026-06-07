import type { ConductorGoal } from "./types";

export function conductorGoalCounts(goal: ConductorGoal): {
  planners: number;
  cards: number;
  agents: number;
} {
  return {
    planners: goal.summary.has_planner_task ? 1 : 0,
    cards: goal.summary.task_count,
    agents: goal.summary.chat_count,
  };
}
