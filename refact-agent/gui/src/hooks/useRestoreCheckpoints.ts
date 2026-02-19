import { useCallback } from "react";
import { Checkpoint } from "../features/Checkpoints/types";
import { checkpointsApi } from "../services/refact/checkpoints";

export const useRestoreCheckpoints = () => {
  const [mutationTrigger, { isLoading }] =
    checkpointsApi.useRestoreCheckpointsMutation();

  const restoreChangesFromCheckpoints = useCallback(
    (checkpoints: Checkpoint[], chat_id: string, chat_mode?: string) => {
      return mutationTrigger({ checkpoints, chat_id, chat_mode });
    },
    [mutationTrigger],
  );

  return { restoreChangesFromCheckpoints, isLoading };
};
