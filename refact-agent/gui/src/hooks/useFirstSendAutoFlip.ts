import { useEffect, useRef } from "react";
import { useAppDispatch, useAppSelector } from "./index";
import {
  selectMessages,
  selectAutoEnrichmentEnabled,
  selectMemoryEnrichmentUserTouched,
  setAutoEnrichmentEnabled,
} from "../features/Chat";
import { selectChatId } from "../features/Chat/Thread/selectors";
import { updateChatParams } from "../services/refact/chatCommands";
import { selectLspPort, selectApiKey } from "../features/Config/configSlice";

export function useFirstSendAutoFlip() {
  const dispatch = useAppDispatch();
  const chatId = useAppSelector(selectChatId);
  const port = useAppSelector(selectLspPort);
  const apiKey = useAppSelector(selectApiKey);
  const messages = useAppSelector(selectMessages);
  const autoEnabled = useAppSelector(selectAutoEnrichmentEnabled);
  const userTouched = useAppSelector(selectMemoryEnrichmentUserTouched);

  const prevUserCountRef = useRef(0);

  useEffect(() => {
    const userCount = messages.filter((m) => m.role === "user").length;

    if (
      prevUserCountRef.current === 0 &&
      userCount === 1 &&
      autoEnabled &&
      !userTouched &&
      chatId &&
      port
    ) {
      dispatch(setAutoEnrichmentEnabled({ chatId, value: false }));
      void updateChatParams(
        chatId,
        { auto_enrichment_enabled: false },
        port,
        apiKey ?? undefined,
      ).catch(() => undefined);
    }

    prevUserCountRef.current = userCount;
  }, [messages, autoEnabled, userTouched, chatId, port, apiKey, dispatch]);
}
