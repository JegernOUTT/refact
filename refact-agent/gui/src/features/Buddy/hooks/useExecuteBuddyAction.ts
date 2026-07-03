import { useCallback } from "react";
import { useAppDispatch } from "../../../hooks";
import { push } from "../../Pages/pagesSlice";
import { navigateFromBuddyPage, routeDraftByKind } from "../executeBuddyAction";
import {
  useAcceptOpportunityMutation,
  useDismissOpportunityMutation,
} from "../../../services/refact/buddy";
import { openBuddyChat, newBuddyChatAction } from "../../Chat/Thread";
import {
  recordVerdictReaction,
  setActiveSpeech,
  setBuddySnapshot,
} from "../buddySlice";
import type {
  BuddyAction,
  BuddyOpportunity,
  BuddyOpportunityAcceptResponse,
  BuddySpeechItem,
} from "../types";

function actionFeedbackSpeech(
  id: string,
  text: string,
  speechIntent: string,
): BuddySpeechItem {
  return {
    id,
    text,
    mood: "happy",
    scope: "opportunity_action",
    persistent: false,
    ttl_seconds: 8,
    dedupe_key: id,
    created_at: new Date().toISOString(),
    controls: [],
    speech_intent: speechIntent,
  };
}

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
  const [dismissOpportunity] = useDismissOpportunityMutation();

  return useCallback(
    async (
      action: BuddyAction,
      opp: BuddyOpportunity | null,
      actionIndex: number,
      options?: { never?: boolean },
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
          const response = await dismissOpportunity({
            id: opp.id,
            never: options?.never === true,
          }).unwrap();
          dispatch(setBuddySnapshot(response.snapshot));
          dispatch(recordVerdictReaction("dismiss"));
        } catch (error) {
          throw new Error(formatOpportunityActionError(error));
        }
        return;
      }

      let response: BuddyOpportunityAcceptResponse;
      try {
        response = await acceptOpportunity({
          id: opp.id,
          action_index: actionIndex,
        }).unwrap();
      } catch (error) {
        throw new Error(formatOpportunityActionError(error));
      }
      dispatch(setBuddySnapshot(response.snapshot));
      dispatch(recordVerdictReaction("accept"));

      const result = response.action_result;
      switch (result.kind) {
        case "open_page":
          navigateFromBuddyPage(result.navigate_to, dispatch);
          break;
        case "launch_investigation_chat":
        case "open_chat":
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
        case "memory_batch_applied":
          if (result.failed > 0) {
            throw new Error(
              `Applied ${result.applied}, ${result.failed} failed (${result.remaining} left)`,
            );
          }
          break;
        case "config_patch_applied":
          dispatch(
            setActiveSpeech(
              actionFeedbackSpeech(
                `config-patch-applied-${result.receipt_id}`,
                `Applied ${result.target_path}. Undo receipt ${result.receipt_id} is available.`,
                "config_patch_applied",
              ),
            ),
          );
          break;
        case "quest_accepted":
          dispatch(
            setActiveSpeech(
              actionFeedbackSpeech(
                `quest-accepted-${result.quest_id}`,
                `Quest accepted! Reward: ${result.reward_xp} XP.`,
                "quest_accept",
              ),
            ),
          );
          break;
      }
    },
    [dispatch, acceptOpportunity, dismissOpportunity],
  );
}
