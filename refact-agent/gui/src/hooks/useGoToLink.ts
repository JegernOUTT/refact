import { useCallback } from "react";
import { useOpenFileInApp } from "./useOpenFileInApp";
import { isAbsolutePath } from "../utils/isAbsolutePath";
import { useAppDispatch } from "./useAppDispatch";
import { popBackTo, push } from "../features/Pages/pagesSlice";
import { useAppSelector } from "./useAppSelector";
import {
  selectIntegration,
  selectChatId,
} from "../features/Chat/Thread/selectors";
import { debugIntegrations } from "../debugConfig";
import {
  newChatAction,
  clearThreadPauseReasons,
  setThreadConfirmationStatus,
} from "../features/Chat/Thread/actions";

export function useGoToLink() {
  const dispatch = useAppDispatch();
  const { openFile } = useOpenFileInApp();
  const maybeIntegration = useAppSelector(selectIntegration);
  const chatId = useAppSelector(selectChatId);

  const handleGoTo = useCallback(
    ({ goto }: { goto?: string }) => {
      if (!goto) return;
      // TODO:  duplicated in smart links.
      const [action, ...payloadParts] = goto.split(":");
      const payload = payloadParts.join(":");
      switch (action.toLowerCase()) {
        case "editor": {
          openFile({ path: payload });
          return;
        }
        case "settings": {
          const isFile = isAbsolutePath(payload);
          debugIntegrations(`[DEBUG]: maybeIntegration: `, maybeIntegration);
          if (!maybeIntegration) {
            debugIntegrations(`[DEBUG]: integration data is not available.`);
            return;
          }
          dispatch(
            popBackTo({
              name: "integrations page",
              // projectPath: isFile ? payload : "",
              integrationName:
                !isFile && payload !== "DEFAULT"
                  ? payload
                  : maybeIntegration.name,
              integrationPath: isFile ? payload : maybeIntegration.path,
              projectPath: maybeIntegration.project,
              shouldIntermediatePageShowUp:
                payload !== "DEFAULT"
                  ? maybeIntegration.shouldIntermediatePageShowUp
                  : false,
              wasOpenedThroughChat: true,
            }),
          );
          // TODO: open in the integrations
          return;
        }

        case "newchat": {
          dispatch(newChatAction());
          dispatch(clearThreadPauseReasons({ id: chatId }));
          dispatch(
            setThreadConfirmationStatus({
              id: chatId,
              wasInteracted: false,
              confirmationStatus: true,
            }),
          );
          dispatch(popBackTo({ name: "history" }));
          dispatch(push({ name: "chat" }));
          return;
        }
        default: {
          // eslint-disable-next-line no-console
          console.log(`[DEBUG]: unexpected action, doing nothing`);
          return;
        }
      }
    },
    [dispatch, chatId, maybeIntegration, openFile],
  );

  return { handleGoTo };
}
