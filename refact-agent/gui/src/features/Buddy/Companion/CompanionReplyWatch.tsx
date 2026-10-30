import { useEffect, useRef } from "react";
import { useAppDispatch, useAppSelector } from "../../../hooks";
import {
  selectIsStreamingById,
  selectIsWaitingById,
  selectMessagesById,
  selectChatErrorById,
} from "../../Chat/Thread";
import {
  registerVisibleChatMount,
  unregisterVisibleChatMount,
} from "../../Connection";

export function CompanionReplyWatch({
  chatId,
  sourceId,
  reading,
  onUnread,
}: {
  chatId: string;
  sourceId: string;
  reading: boolean;
  onUnread: (sourceId: string) => void;
}) {
  const dispatch = useAppDispatch();
  const busy = useAppSelector(
    (state) =>
      selectIsStreamingById(state, chatId) ||
      selectIsWaitingById(state, chatId),
  );
  const replyId = useAppSelector((state) => {
    const message = [...selectMessagesById(state, chatId)]
      .reverse()
      .find((value) => value.role === "assistant");
    return message?.message_id ?? null;
  });
  const error = useAppSelector((state) => selectChatErrorById(state, chatId));
  const previous = useRef({ busy, replyId });
  const callback = useRef(onUnread);
  callback.current = onUnread;
  useEffect(() => {
    if (!busy) return;
    dispatch(registerVisibleChatMount({ chatId }));
    return () => {
      dispatch(unregisterVisibleChatMount({ chatId }));
    };
  }, [busy, chatId, dispatch]);
  useEffect(() => {
    if (
      !error &&
      !reading &&
      previous.current.busy &&
      !busy &&
      replyId !== null
    )
      callback.current(sourceId);
    previous.current = { busy, replyId };
  }, [busy, replyId, reading, sourceId, error]);
  return null;
}
