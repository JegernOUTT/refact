import { useEffect, useRef } from "react";
import { useAppDispatch, useAppSelector } from "./index";
import {
  selectMessagesById,
  selectAutoEnrichmentEnabledById,
  selectMemoryEnrichmentUserTouchedById,
  setAutoEnrichmentEnabled,
  useThreadId,
} from "../features/Chat/Thread";
import { updateChatParams } from "../services/refact/chatCommands";
import { selectConfig, selectApiKey } from "../features/Config/configSlice";

export function useFirstSendAutoFlip() {
  const dispatch = useAppDispatch();
  const chatId = useThreadId();
  const config = useAppSelector(selectConfig);
  const apiKey = useAppSelector(selectApiKey);
  const messages = useAppSelector((state) => selectMessagesById(state, chatId));
  const autoEnabled = useAppSelector((state) =>
    selectAutoEnrichmentEnabledById(state, chatId),
  );
  const userTouched = useAppSelector((state) =>
    selectMemoryEnrichmentUserTouchedById(state, chatId),
  );

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
