import { useCallback } from "react";
import { useAppDispatch } from "../../../hooks";
import { push } from "../../Pages/pagesSlice";
import { navigateFromBuddyPage, routeDraftByKind } from "../executeBuddyAction";
import { validateBuddyDraftAction } from "../buddyActionValidation";
import {
  useAcceptOpportunityMutation,
  useCreateConductorGoalMutation,
  useDismissOpportunityMutation,
} from "../../../services/refact/buddy";
import { openBuddyChat, newBuddyChatAction } from "../../Chat/Thread";
import { setBuddySnapshot } from "../buddySlice";
import type {
  BuddyAction,
  BuddyOpportunity,
  BuddyOpportunityAcceptResponse,
  CreateConductorGoalRequest,
} from "../types";

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function stringFromUnknown(value: unknown): string | null {
  if (typeof value === "string") return value;
  if (value instanceof Error) return value.message;
  if (isRecord(value)) {
    if (typeof value.detail === "string") return value.detail;
    if (typeof value.message === "string") return value.message;
    if (typeof value.error === "string") return value.error;
  }
  return null;
}

export function formatOpportunityActionError(error: unknown): string {
  const direct = stringFromUnknown(error);
  if (direct) return direct;
  if (isRecord(error)) {
    const status =
      typeof error.status === "number" || typeof error.status === "string"
        ? String(error.status)
        : null;
    const data = stringFromUnknown(error.data);
    const nested = stringFromUnknown(error.error);
    const message = data ?? nested;
    if (status && message) return `${status}: ${message}`;
    if (message) return message;
    if (status) return `Action failed (${status})`;
  }
  return "Action failed. Please try again.";
}

export function useExecuteBuddyAction() {
  const dispatch = useAppDispatch();
  const [acceptOpportunity] = useAcceptOpportunityMutation();
  const [createConductorGoal] = useCreateConductorGoalMutation();
  const [dismissOpportunity] = useDismissOpportunityMutation();

  return useCallback(
    async (
      action: BuddyAction,
      opp: BuddyOpportunity | null,
      actionIndex: number,
      conductorGoal?: CreateConductorGoalRequest,
    ) => {
      if (opp == null) {
        if (action.kind === "open_page") {
          navigateFromBuddyPage(action.page, dispatch);
          return;
        }
        return;
      }

      if (action.kind === "dismiss") {
        try {
          const response = await dismissOpportunity(opp.id).unwrap();
          dispatch(setBuddySnapshot(response.snapshot));
        } catch (error) {
          throw new Error(formatOpportunityActionError(error));
        }
        return;
      }

      const validationError = validateBuddyDraftAction(action);
      if (validationError) {
        throw new Error(validationError);
      }

      let createdGoalId: string | undefined;
      if (action.kind === "start_conductor_goal") {
        if (!conductorGoal) {
          throw new Error("Complete the conductor goal details first.");
        }
        try {
          const goal = await createConductorGoal(conductorGoal).unwrap();
          createdGoalId = goal.id;
        } catch (error) {
          throw new Error(formatOpportunityActionError(error));
        }
      }

      let response: BuddyOpportunityAcceptResponse;
      try {
        response = await acceptOpportunity({
          id: opp.id,
          action_index: actionIndex,
          ...(createdGoalId ? { created_goal_id: createdGoalId } : {}),
        }).unwrap();
      } catch (error) {
        throw new Error(formatOpportunityActionError(error));
      }
      dispatch(setBuddySnapshot(response.snapshot));

      const result = response.action_result;
      switch (result.kind) {
        case "open_page":
          navigateFromBuddyPage(result.navigate_to, dispatch);
          break;
        case "launch_investigation_chat":
          dispatch(newBuddyChatAction({ chat_id: result.chat_id }));
          dispatch(openBuddyChat({ chat_id: result.chat_id }));
          dispatch(push({ name: "chat" }));
          break;
        case "draft":
          routeDraftByKind(result, dispatch);
          break;
        case "dismiss":
          break;
        case "marketplace_install":
          if (result.success === false) {
            throw new Error(result.error ?? "Marketplace install failed");
          }
          dispatch(push({ name: "marketplace hub" }));
          break;
        case "start_conductor_goal":
          dispatch(push({ name: "conductor" }));
          break;
      }
    },
    [dispatch, acceptOpportunity, createConductorGoal, dismissOpportunity],
  );
}
