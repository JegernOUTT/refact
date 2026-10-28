import { createSelector } from "@reduxjs/toolkit";
import type { RootState } from "../../app/store";
import { selectMessagesById } from "../../features/Chat/Thread/selectors";
import type { EventMessage } from "../../services/refact";
import { foldedEventIndexFor } from "./foldedEventIndex";

export const selectFoldedEventIndexById = createSelector(
  [selectMessagesById],
  foldedEventIndexFor,
);

export function selectFoldedProcessEvent(
  state: RootState,
  threadId: string,
  toolCallId: string | undefined,
): EventMessage | undefined {
  if (!toolCallId) return undefined;
  return selectFoldedEventIndexById(
    state,
    threadId,
  ).processEventByToolCallId.get(toolCallId);
}

export function selectFoldedAgentEvent(
  state: RootState,
  threadId: string,
  toolCallId: string | undefined,
): EventMessage | undefined {
  if (!toolCallId) return undefined;
  return selectFoldedEventIndexById(state, threadId).agentEventByToolCallId.get(
    toolCallId,
  );
}
