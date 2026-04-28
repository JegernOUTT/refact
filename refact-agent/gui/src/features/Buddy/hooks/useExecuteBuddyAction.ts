import { useCallback } from "react";
import { useAppDispatch } from "../../../hooks";
import { push } from "../../Pages/pagesSlice";
import { executeBuddyNavigation } from "../executeBuddyAction";
import {
  useAcceptOpportunityMutation,
  useDismissOpportunityMutation,
  useLaunchInvestigationMutation,
} from "../../../services/refact/buddy";
import { openBuddyChat, newBuddyChatAction } from "../../Chat/Thread";
import type { BuddyAction, BuddyOpportunity } from "../types";

export function useExecuteBuddyAction() {
  const dispatch = useAppDispatch();
  const [acceptOpportunity] = useAcceptOpportunityMutation();
  const [dismissOpportunity] = useDismissOpportunityMutation();
  const [launchInvestigation] = useLaunchInvestigationMutation();

  return useCallback(
    async (action: BuddyAction, opp: BuddyOpportunity | null) => {
      const actionIndex =
        opp != null ? opp.proposed_actions.findIndex((a) => a === action) : -1;

      switch (action.kind) {
        case "open_page":
          executeBuddyNavigation(action.page, dispatch);
          break;

        case "launch_investigation_chat": {
          const result = await launchInvestigation(action.preload).unwrap();
          dispatch(newBuddyChatAction({ chat_id: result.chat_id }));
          dispatch(openBuddyChat({ chat_id: result.chat_id }));
          dispatch(push({ name: "chat" }));
          break;
        }

        case "draft_skill":
          dispatch(
            push({
              name: "extensions",
              tab: "skills",
              draftId: action.draft_id,
            }),
          );
          break;

        case "draft_command":
          dispatch(
            push({
              name: "extensions",
              tab: "commands",
              draftId: action.draft_id,
            }),
          );
          break;

        case "draft_subagent":
          dispatch(
            push({
              name: "customization",
              kind: "subagents",
              draftId: action.draft_id,
            }),
          );
          break;

        case "draft_mode":
          dispatch(
            push({
              name: "customization",
              kind: "modes",
              draftId: action.draft_id,
            }),
          );
          break;

        case "draft_agents_md_patch":
          dispatch(push({ name: "customization" }));
          break;

        case "draft_defaults_change":
          dispatch(push({ name: "default models" }));
          break;

        case "draft_customization_change":
          dispatch(push({ name: "customization" }));
          break;

        case "offer_marketplace_install":
          dispatch(push({ name: "marketplace hub" }));
          break;

        case "create_pulse_report":
          break;

        case "dismiss":
          if (opp != null) await dismissOpportunity(opp.id).unwrap();
          return;
      }

      if (opp != null && actionIndex >= 0) {
        void acceptOpportunity(opp.id);
      }
    },
    [dispatch, acceptOpportunity, dismissOpportunity, launchInvestigation],
  );
}
