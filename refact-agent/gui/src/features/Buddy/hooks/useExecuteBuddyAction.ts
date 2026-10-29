import { useCallback } from "react";
import { useAppDispatch, useAppSelector } from "../../../hooks";
import {
  buddyOperatorIdentity,
  buddyOperatorApi,
} from "../../../services/refact/buddyOperator";
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

export type BuddyControlActionResult =
  | { status: "success" }
  | {
      status: "partial";
      applied: number;
      failed: number;
      remaining: number;
      message: string;
    }
  | { status: "unsupported"; message: string }
  | { status: "failed"; message: string }
  | { status: "ignored"; reason: "duplicate" | "stale" };

export const BUDDY_ACTION_SUCCESS: BuddyControlActionResult = {
  status: "success",
};

export function isBuddyActionApplied(
  result: BuddyControlActionResult,
): result is { status: "success" } {
  return result.status === "success";
}

export function buddyActionResultMessage(
  result: BuddyControlActionResult,
): string | null {
  switch (result.status) {
    case "partial":
    case "unsupported":
    case "failed":
      return result.message;
    case "success":
    case "ignored":
      return null;
  }
}

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
  const identity = useAppSelector(buddyOperatorIdentity);
  const [acceptOpportunity] = useAcceptOpportunityMutation();
  const [dismissOpportunity] = useDismissOpportunityMutation();

  return useCallback(
    async (
      action: BuddyAction,
      opp: BuddyOpportunity | null,
      actionIndex: number,
      options?: { never?: boolean },
      isCurrent: () => boolean = () => true,
    ): Promise<BuddyControlActionResult> => {
      if (opp == null) {
        if (action.kind === "open_page") {
          if (!isCurrent()) return { status: "ignored", reason: "stale" };
          navigateFromBuddyPage(action.page, dispatch);
          return BUDDY_ACTION_SUCCESS;
        }
        return {
          status: "unsupported",
          message:
            "This action needs an opportunity to act on and is unavailable here.",
        };
      }

      const authority = await dispatch(
        buddyOperatorApi.endpoints.getBuddyOperatorStatus.initiate(identity, {
          subscribe: false,
          forceRefetch: true,
        }),
      );
      if (!isCurrent()) return { status: "ignored", reason: "stale" };
      if (Boolean(authority.error) || authority.data?.mode !== "disabled") {
        return {
          status: "unsupported",
          message:
            "Open this proposal in Reports to use its current actions. Older proposal actions only work when Reports storage is turned off.",
        };
      }

      if (action.kind === "dismiss") {
        try {
          const response = await dismissOpportunity({
            id: opp.id,
            never: options?.never === true,
          }).unwrap();
          if (!isCurrent()) return { status: "ignored", reason: "stale" };
          dispatch(setBuddySnapshot(response.snapshot));
          dispatch(recordVerdictReaction("dismiss"));
        } catch (error) {
          return {
            status: "failed",
            message: formatOpportunityActionError(error),
          };
        }
        return BUDDY_ACTION_SUCCESS;
      }

      let response: BuddyOpportunityAcceptResponse;
      try {
        response = await acceptOpportunity({
          id: opp.id,
          action_index: actionIndex,
        }).unwrap();
      } catch (error) {
        return {
          status: "failed",
          message: formatOpportunityActionError(error),
        };
      }
      if (!isCurrent()) return { status: "ignored", reason: "stale" };
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
            return {
              status: "failed",
              message: result.error ?? "Marketplace install failed",
            };
          }
          dispatch(push({ name: "marketplace hub" }));
          break;
        case "memory_batch_applied":
          if (result.failed > 0) {
            return {
              status: "partial",
              applied: result.applied,
              failed: result.failed,
              remaining: result.remaining,
              message: `Applied ${result.applied}, ${result.failed} failed (${result.remaining} left)`,
            };
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
        default:
          return {
            status: "unsupported",
            message:
              "The server returned an unsupported action result. No success was inferred.",
          };
      }
      return BUDDY_ACTION_SUCCESS;
    },
    [dispatch, acceptOpportunity, dismissOpportunity, identity],
  );
}
