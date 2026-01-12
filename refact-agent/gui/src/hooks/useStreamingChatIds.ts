import { useMemo } from "react";
import {
  useListTrajectoriesQuery,
  TrajectoryMeta,
} from "../services/refact/trajectories";

export type SessionState = NonNullable<TrajectoryMeta["session_state"]>;

export function useChatSessionStates(): Record<string, SessionState> {
  const { data: trajectories } = useListTrajectoriesQuery(undefined, {
    pollingInterval: 2000,
  });

  return useMemo(() => {
    if (!trajectories) return {};
    const states: Record<string, SessionState> = {};
    for (const t of trajectories) {
      if (t.session_state) {
        states[t.id] = t.session_state;
      }
    }
    return states;
  }, [trajectories]);
}
