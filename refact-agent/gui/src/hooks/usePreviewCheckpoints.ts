import { useCallback } from "react";
import { Checkpoint } from "../features/Checkpoints/types";
import { checkpointsApi } from "../services/refact/checkpoints";

export const usePreviewCheckpoints = () => {
  const [mutationTrigger, { isLoading }] =
    checkpointsApi.usePreviewCheckpointsMutation();

  const previewChangesFromCheckpoints = useCallback(
    (checkpoints: Checkpoint[], chat_id: string, chat_mode?: string) => {
      return mutationTrigger({ checkpoints, chat_id, chat_mode });
    },
    [mutationTrigger],
  );

  return { previewChangesFromCheckpoints, isLoading };
};
