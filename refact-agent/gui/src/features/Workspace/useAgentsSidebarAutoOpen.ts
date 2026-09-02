import { useEffect, useRef } from "react";

import { useMediaQuery } from "../../components/ui";
import { useAppDispatch, useAppSelector } from "../../hooks";
import {
  autoOpenRequested,
  selectAgentsPanelAutoOpenedFor,
  selectAgentsPanelUserClosed,
} from "../AgentsPanel/agentsPanelSlice";
import { selectActiveBackgroundAgents } from "../Chat/Thread";
import {
  selectFocusedWorkspaceChatId,
  selectWorkspaceDock,
  setDockOpen,
  setDockSection,
} from "./workspaceSlice";

const narrowQuery = "(max-width: 767px)";

export function useAgentsSidebarAutoOpen() {
  const dispatch = useAppDispatch();
  const isNarrow = useMediaQuery(narrowQuery);
  const focusedChatId = useAppSelector(selectFocusedWorkspaceChatId);
  const dock = useAppSelector(selectWorkspaceDock);
  const autoOpenedFor = useAppSelector(selectAgentsPanelAutoOpenedFor);
  const userClosed = useAppSelector((state) =>
    focusedChatId ? selectAgentsPanelUserClosed(state, focusedChatId) : false,
  );
  const activeAgents = useAppSelector((state) =>
    focusedChatId ? selectActiveBackgroundAgents(state, focusedChatId) : null,
  );
  const runningCount = activeAgents?.length ?? 0;
  const previousRef = useRef<{ chatId: string | null; running: number }>({
    chatId: focusedChatId,
    running: runningCount,
  });

  useEffect(() => {
    const previous = previousRef.current;
    previousRef.current = { chatId: focusedChatId, running: runningCount };
    if (!focusedChatId || previous.chatId !== focusedChatId) return;

    if (previous.running === 0 && runningCount > 0) {
      if (isNarrow || dock.open || userClosed) return;
      dispatch(setDockSection("agents"));
      dispatch(setDockOpen(true));
      dispatch(autoOpenRequested(focusedChatId));
      return;
    }

    if (
      previous.running > 0 &&
      runningCount === 0 &&
      autoOpenedFor === focusedChatId &&
      dock.open &&
      dock.section === "agents"
    ) {
      dispatch(setDockOpen(false));
    }
  }, [
    autoOpenedFor,
    dispatch,
    dock.open,
    dock.section,
    focusedChatId,
    isNarrow,
    runningCount,
    userClosed,
  ]);
}
