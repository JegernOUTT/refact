import { useCallback } from "react";
import { useAppDispatch } from "../../../hooks";
import { push } from "../../Pages/pagesSlice";
import { navigateFromBuddyPage, routeDraftByKind } from "../executeBuddyAction";
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
          navigateFromBuddyPage(action.page, dispatch);
          return;
        }
        return;
      }

      if (action.kind === "dismiss") {
        await dismissOpportunity(opp.id).unwrap();
        return;
      }

      const response = await acceptOpportunity({
        id: opp.id,
        action_index: actionIndex,
      }).unwrap();
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
          dispatch(push({ name: "marketplace hub" }));
          break;
        case "unimplemented":
          break;
      }
    },
    [dispatch, acceptOpportunity, dismissOpportunity],
  );
}
