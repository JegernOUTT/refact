import { useCallback } from "react";
import { useAppSelector } from "./useAppSelector";
import { selectConfig, selectApiKey } from "../features/Config/configSlice";
import {
  sendChatCommand,
  type ChatCommandBase,
} from "../services/refact/chatCommands";

export function useSendChatCommand() {
  const config = useAppSelector(selectConfig);
  const apiKey = useAppSelector(selectApiKey);

  return useCallback(
    async (chatId: string, command: ChatCommandBase) => {
      await sendChatCommand(chatId, config, apiKey ?? undefined, command);
    },
    [config, apiKey],
  );
}
