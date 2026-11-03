import { useAppSelector } from "../../hooks/useAppSelector";
import {
  selectHasMessagesById,
  selectIsStreamingById,
  selectIsWaitingById,
  selectSnapshotReceivedById,
} from "../../features/Chat/Thread/selectors";
import { selectSseStatusForChat } from "../../features/Connection/connectionSlice";

export type ComposerActivity = "loading" | "working" | null;

export function useComposerActivity(chatId: string): ComposerActivity {
  return useAppSelector((state): ComposerActivity => {
    const sse = selectSseStatusForChat(state, chatId);
    if (
      sse !== "disconnected" &&
      !selectHasMessagesById(state, chatId) &&
      (!selectSnapshotReceivedById(state, chatId) || sse === "connecting")
    ) {
      return "loading";
    }
    if (
      selectIsStreamingById(state, chatId) ||
      selectIsWaitingById(state, chatId)
    ) {
      return "working";
    }
    return null;
  });
}
