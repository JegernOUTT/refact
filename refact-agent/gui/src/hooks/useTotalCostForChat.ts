import { selectMessagesById, useThreadId } from "../features/Chat/Thread";
import {
  getTotalTokenMeteringForMessages,
  getTotalUsdMeteringForMessages,
} from "../utils/getMetering";
import { useAppSelector } from "./useAppSelector";

export const useTotalTokenMeteringForChat = () => {
  const chatId = useThreadId();
  const messages = useAppSelector((state) => selectMessagesById(state, chatId));
  return getTotalTokenMeteringForMessages(messages);
};

export const useTotalUsdForChat = () => {
  const chatId = useThreadId();
  const messages = useAppSelector((state) => selectMessagesById(state, chatId));
  return getTotalUsdMeteringForMessages(messages);
};
