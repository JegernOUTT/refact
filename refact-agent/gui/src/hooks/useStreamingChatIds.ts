import { useMemo } from "react";
import { useListTrajectoriesQuery } from "../services/refact/trajectories";

export function useStreamingChatIds(): Set<string> {
  const { data: trajectories } = useListTrajectoriesQuery(undefined, {
    pollingInterval: 2000,
  });

  return useMemo(() => {
    if (!trajectories) return new Set<string>();
    return new Set(
      trajectories.filter((t) => t.is_streaming).map((t) => t.id),
    );
  }, [trajectories]);
}
