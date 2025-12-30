import { useCallback } from "react";
import { selectThread } from "../features/Chat/Thread/selectors";
import { useAppSelector } from "./useAppSelector";
import { ChatMessages, knowledgeApi } from "../services/refact";
import { newChatAction } from "../events";
import { useAppDispatch } from "./useAppDispatch";
import { setError } from "../features/Errors/errorsSlice";
import { setIsWaitingForResponse } from "../features/Chat";

export function useCompressChat() {
  const dispatch = useAppDispatch();
  const thread = useAppSelector(selectThread);

  const [submit, request] = knowledgeApi.useCompressMessagesMutation({
    fixedCacheKey: thread?.id ?? "",
  });

  const compressChat = useCallback(async () => {
    if (!thread) return;

    dispatch(setIsWaitingForResponse({ id: thread.id, value: true }));
    const result = await submit({
      messages: thread.messages,
      project: thread.project_name ?? "",
    });
    dispatch(setIsWaitingForResponse({ id: thread.id, value: false }));

    if (result.error) {
      // TODO: handle errors
      dispatch(
        setError("Error compressing chat: " + JSON.stringify(result.error)),
      );
    }

    if (result.data) {
      const content =
        "🗜️ I am continuing from a compressed chat history. Here is what happened so far: " +
        result.data.trajectory;
      const messages: ChatMessages = [{ role: "user", content }];

      dispatch(newChatAction({ messages, title: `🗜️ ${thread.title}` }));
    }
  }, [dispatch, submit, thread]);

  return {
    compressChat,
    compressChatRequest: request,
    isCompressing: request.isLoading,
  };
}
