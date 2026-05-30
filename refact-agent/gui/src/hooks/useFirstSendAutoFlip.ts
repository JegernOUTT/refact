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
import { selectConfig, selectApiKey } from "../features/Config/configSlice";

export function useFirstSendAutoFlip() {
  const dispatch = useAppDispatch();
  const chatId = useAppSelector(selectChatId);
  const config = useAppSelector(selectConfig);
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
      chatId
    ) {
      dispatch(setAutoEnrichmentEnabled({ chatId, value: false }));
      void updateChatParams(
        chatId,
        { auto_enrichment_enabled: false },
        config,
        apiKey ?? undefined,
      ).catch(() => undefined);
    }

    prevUserCountRef.current = userCount;
  }, [messages, autoEnabled, userTouched, chatId, config, apiKey, dispatch]);
}
