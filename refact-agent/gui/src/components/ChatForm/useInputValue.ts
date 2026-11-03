import React, { useCallback, useEffect } from "react";
import { useAppDispatch } from "../../hooks/useAppDispatch";
import { useAppSelector } from "../../hooks/useAppSelector";
import { selectPages, change, ChatPage } from "../../features/Pages/pagesSlice";
import { setInputValue, addInputValue } from "./actions";
import { debugRefact } from "../../debugConfig";
import { useDraftMessage } from "../../hooks/useDraftMessage";
import { sendIdeMessagesToCurrentChat } from "../../features/Chat/Thread/actions";
import { selectIsBuddyChat, useThreadId } from "../../features/Chat/Thread";
import { isHostMessage } from "../../utils/isHostMessage";

export function useInputValue(
  uncheckCheckboxes: () => void,
): [
  string,
  React.Dispatch<React.SetStateAction<string>>,
  boolean,
  React.Dispatch<React.SetStateAction<boolean>>,
] {
  const [isSendImmediately, setIsSendImmediately] =
    React.useState<boolean>(false);
  const dispatch = useAppDispatch();
  const pages = useAppSelector(selectPages);
  const chatId = useThreadId();
  const isBuddyChat = useAppSelector((state) =>
    selectIsBuddyChat(state, chatId),
  );

  const setUpIfNotReady = useCallback(() => {
    const lastPage = pages[pages.length - 1];
    if (lastPage.name !== "chat") {
      const chatPage: ChatPage = { name: "chat" };
      dispatch(change(chatPage));
    }
  }, [dispatch, pages]);

  const handleQueuedMessageEdit = useCallback(
    (previousValue: string) => {
      setUpIfNotReady();
      if (!previousValue.trim()) uncheckCheckboxes();
      setIsSendImmediately(false);
    },
    [setUpIfNotReady, uncheckCheckboxes],
  );

  const { value, setValue } = useDraftMessage(handleQueuedMessageEdit);

  const handleEvent = useCallback(
    (event: MessageEvent) => {
      const isSameWindowPost =
        event.source === window && window.location.origin !== "null";
      const isSameOrigin =
        window.location.origin !== "null" &&
        event.origin === window.location.origin;
      if (isSameWindowPost && !isSameOrigin) {
        return;
      }

      if (!isHostMessage(event)) return;
      if (addInputValue.match(event.data) || setInputValue.match(event.data)) {
        const { payload } = event.data;
        if (payload.chatId !== undefined && payload.chatId !== chatId) return;
        debugRefact(
          `[DEBUG]: receiving event setInputValue/addInputValue with payload:`,
          payload,
        );
        setUpIfNotReady();

        if (payload.messages && payload.messages.length > 0) {
          debugRefact(`[DEBUG]: payload messages: `, payload.messages);
          setIsSendImmediately(payload.send_immediately);
          void dispatch(
            sendIdeMessagesToCurrentChat({
              chatId,
              messages: payload.messages,
              priority: payload.send_immediately,
            }),
          );
          return;
        }
      }

      if (addInputValue.match(event.data)) {
        const { payload } = event.data;
        debugRefact(`[DEBUG]: addInputValue triggered with:`, payload);
        const { send_immediately, value } = payload;
        setValue((prev) => {
          debugRefact(`[DEBUG]: Previous value: "${prev}", Adding: "${value}"`);
          return prev + value;
        });
        setIsSendImmediately(send_immediately);
        return;
      }

      if (setInputValue.match(event.data)) {
        const { payload } = event.data;
        debugRefact(`[DEBUG]: setInputValue triggered with:`, payload);
        const { send_immediately, value } = payload;
        uncheckCheckboxes();
        setValue(value ?? "");
        debugRefact(`[DEBUG]: setInputValue.payload: `, payload);
        setIsSendImmediately(send_immediately);
        return;
      }
    },
    [setUpIfNotReady, dispatch, uncheckCheckboxes, setValue, chatId],
  );

  useEffect(() => {
    if (isBuddyChat) return;
    window.addEventListener("message", handleEvent);

    return () => window.removeEventListener("message", handleEvent);
  }, [handleEvent, isBuddyChat]);

  return [value, setValue, isSendImmediately, setIsSendImmediately];
}
