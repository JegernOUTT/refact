import { useCallback } from "react";
import { useAppDispatch } from "../../../hooks";
import { push } from "../../Pages/pagesSlice";
import { executeBuddyNavigation } from "../executeBuddyAction";
import {
  useAcceptOpportunityMutation,
  useDismissOpportunityMutation,
} from "../../../services/refact/buddy";
import { openBuddyChat, newBuddyChatAction } from "../../Chat/Thread";
import { setBuddySnapshot } from "../buddySlice";
import type { BuddyAction, BuddyOpportunity } from "../types";

export function useExecuteBuddyAction() {
  const dispatch = useAppDispatch();
  const [acceptOpportunity] = useAcceptOpportunityMutation();
  const [dismissOpportunity] = useDismissOpportunityMutation();

  return useCallback(
    async (
      action: BuddyAction,
      opp: BuddyOpportunity | null,
      actionIndex: number,
    ) => {
      if (opp == null) {
        if (action.kind === "open_page") {
          executeBuddyNavigation(action.page, dispatch);
          return;
        }
        console.warn("workshop action not supported:", action.kind);
        return;
      }

      if (action.kind === "dismiss") {
        try {
          await dismissOpportunity(opp.id).unwrap();
        } catch (err) {
          console.error("buddy: dismiss failed", err);
          throw err;
        }
        return;
      }

      try {
        const response = await acceptOpportunity({
          id: opp.id,
          action_index: actionIndex,
        }).unwrap();
        dispatch(setBuddySnapshot(response.snapshot));

        const result = response.action_result;
        switch (result.kind) {
          case "open_page":
            executeBuddyNavigation(result.navigate_to, dispatch);
            break;
          case "launch_investigation_chat":
            dispatch(newBuddyChatAction({ chat_id: result.chat_id }));
            dispatch(openBuddyChat({ chat_id: result.chat_id }));
            dispatch(push({ name: "chat" }));
            break;
          case "draft":
            switch (result.draft_kind) {
              case "skill":
                dispatch(
                  push({
                    name: "extensions",
                    tab: "skills",
                    draftId: result.draft_id,
                  }),
                );
                break;
              case "command":
                dispatch(
                  push({
                    name: "extensions",
                    tab: "commands",
                    draftId: result.draft_id,
                  }),
                );
                break;
              case "delegate":
                dispatch(
                  push({
                    name: "customization",
                    kind: "subagents",
                    draftId: result.draft_id,
                  }),
                );
                break;
              case "mode":
                dispatch(
                  push({
                    name: "customization",
                    kind: "modes",
                    draftId: result.draft_id,
                  }),
                );
                break;
              case "agents_md":
                dispatch(push({ name: "customization" }));
                break;
              case "defaults_model":
                dispatch(
                  push({ name: "default models", draftId: result.draft_id }),
                );
                break;
              case "hook":
                dispatch(
                  push({
                    name: "extensions",
                    tab: "hooks",
                    draftId: result.draft_id,
                  }),
                );
                break;
            }
            break;
          case "dismiss":
            break;
          case "marketplace_install":
            dispatch(push({ name: "marketplace hub" }));
            break;
          case "unimplemented":
            console.warn("buddy: action unimplemented:", result.action);
            break;
        }
      } catch (err) {
        console.error("buddy: accept failed", err);
        throw err;
      }
    },
    [dispatch, acceptOpportunity, dismissOpportunity],
  );
}
