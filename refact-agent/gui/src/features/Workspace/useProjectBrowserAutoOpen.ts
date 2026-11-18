import { addListener } from "@reduxjs/toolkit";
import { useEffect } from "react";
import type { RootState } from "../../app/store";
import { useAppDispatch } from "../../hooks";
import { browserApi } from "../../services/refact/browser";
import type { ChatEventEnvelope } from "../../services/refact/chatSubscription";
import { applyChatEvent } from "../Chat/Thread/actions";
import { selectSnapshotReceivedById } from "../Chat/Thread/selectors";
import {
  openBrowserForLiveTool,
  selectBrowserAutoOpenHandled,
  selectBrowserSurfaceOpen,
} from "./workspaceSlice";

export function projectBrowserOpenHint(event: ChatEventEnvelope) {
  if (event.type !== "message_added" && event.type !== "message_updated")
    return undefined;
  if (event.message.role !== "tool") return undefined;
  const hint = event.message.extra?.browser_project;
  if (
    typeof hint !== "object" ||
    !hint ||
    !("open_browser_tab" in hint) ||
    hint.open_browser_tab !== true
  )
    return undefined;
  return {
    decisionRequired:
      "decision_required" in hint && hint.decision_required === true,
  };
}

export function useProjectBrowserAutoOpen() {
  const dispatch = useAppDispatch();
  const [getStatus] = browserApi.useProjectBrowserStatusMutation();

  useEffect(() => {
    let disposed = false;
    let inFlight = false;
    const unsubscribe = dispatch(
      addListener({
        actionCreator: applyChatEvent,
        effect: async ({ payload: event }, api) => {
          const hint = projectBrowserOpenHint(event);
          if (!hint || inFlight) return;
          const state = api.getOriginalState() as RootState;
          if (
            selectBrowserAutoOpenHandled(state) ||
            selectBrowserSurfaceOpen(state) ||
            !selectSnapshotReceivedById(state, event.chat_id)
          )
            return;
          inFlight = true;
          try {
            const result = await getStatus(undefined);
            if (disposed || !result.data?.project_id) return;
            const verified = hint.decisionRequired
              ? result.data.migration.state === "available"
              : result.data.running && result.data.connected;
            if (verified) dispatch(openBrowserForLiveTool());
          } finally {
            inFlight = false;
          }
        },
      }),
    );
    return () => {
      disposed = true;
      unsubscribe();
    };
  }, [dispatch, getStatus]);
}
