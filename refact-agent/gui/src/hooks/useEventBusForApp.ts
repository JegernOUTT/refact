import { useEffect, useRef } from "react";
import { useAppSelector } from "./useAppSelector";
import { useAppDispatch } from "./useAppDispatch";
import { useConfig } from "./useConfig";
import { updateConfig } from "../features/Config/configSlice";
import { setFileInfo } from "../features/Chat/activeFile";
import { setSelectedSnippet } from "../features/Chat/selectedSnippet";
import { setCurrentProjectInfo } from "../features/Chat/currentProject";
import {
  newChatAction,
  newChatWithInitialMessages,
} from "../features/Chat/Thread/actions";
import {
  isPageInHistory,
  push,
  selectPages,
} from "../features/Pages/pagesSlice";
import { ideToolCallResponse, ideSwitchToThread } from "./useEventBusForIDE";
import { createAction } from "@reduxjs/toolkit";
import { switchToThread } from "../features/Chat/Thread/actions";
import { usePostUserAction } from "./usePostUserAction";
import {
  ideLogLines,
  normalizeIdeLogLines,
  setIdeLogEntries,
} from "../features/BugReport/ideLog";

export const ideAttachFileToChat = createAction<string>("ide/attachFileToChat");

export function useEventBusForApp() {
  const config = useConfig();
  const dispatch = useAppDispatch();
  const pages = useAppSelector(selectPages);
  const { postFileOpened, postSnippetSelected } = usePostUserAction();
  const pagesRef = useRef(pages);
  pagesRef.current = pages;

  useEffect(() => {
    const listener = (event: MessageEvent) => {
      if (updateConfig.match(event.data)) {
        dispatch(updateConfig(event.data.payload));
      }

      if (setFileInfo.match(event.data)) {
        dispatch(setFileInfo(event.data.payload));
        postFileOpened(event.data.payload.path);
      }

      if (setSelectedSnippet.match(event.data)) {
        dispatch(setSelectedSnippet(event.data.payload));
        const snippet = event.data.payload;
        const line1 = snippet.start_line ?? 1;
        const line2 = snippet.end_line ?? snippet.code.split("\n").length;
        postSnippetSelected(snippet.path, line1, line2);
      }

      if (newChatAction.match(event.data)) {
        if (!isPageInHistory({ pages: pagesRef.current }, "chat")) {
          dispatch(push({ name: "chat" }));
        }
        const payload = event.data.payload;
        if (payload?.messages && payload.messages.length > 0) {
          void dispatch(
            newChatWithInitialMessages({
              title: payload.title,
              messages: payload.messages,
            }),
          );
        } else {
          dispatch(newChatAction(payload));
        }
      }

      if (setCurrentProjectInfo.match(event.data)) {
        dispatch(setCurrentProjectInfo(event.data.payload));
      }

      if (ideToolCallResponse.match(event.data)) {
        dispatch(event.data);
      }

      if (ideSwitchToThread.match(event.data)) {
        if (!isPageInHistory({ pages: pagesRef.current }, "chat")) {
          dispatch(push({ name: "chat" }));
        }
        dispatch(switchToThread({ id: event.data.payload.chatId }));
      }

      if (ideLogLines.match(event.data)) {
        setIdeLogEntries(normalizeIdeLogLines(event.data.payload.lines));
      }

      // TODO: ideToolEditResponse.

      // TODO: active project
      // vscode workspace can be found with vscode.workspace.name
      // JB: project.name
    };

    window.addEventListener("message", listener);

    return () => {
      window.removeEventListener("message", listener);
    };
  }, [config.host, dispatch, postFileOpened, postSnippetSelected]);
}
