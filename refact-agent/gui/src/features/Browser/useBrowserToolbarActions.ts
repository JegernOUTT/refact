import { useEffect, useCallback, useRef } from "react";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { browserApi } from "../../services/refact/browser";
import {
  selectBrowserRuntime,
  shiftPendingToolbarAction,
  updateBrowserFrame,
  setPickerActive,
} from "./browserSlice";
import { selectLspPort, selectApiKey } from "../Config/configSlice";
import { addThreadImage } from "../Chat/Thread/actions";
import { sendUserMessage } from "../../services/refact/chatCommands";

export function useBrowserToolbarActions(chatId: string) {
  const dispatch = useAppDispatch();
  const runtime = useAppSelector((state) =>
    selectBrowserRuntime(state, chatId),
  );
  const port = useAppSelector(selectLspPort);
  const apiKey = useAppSelector(selectApiKey);
  const pendingActions = runtime?.pending_toolbar_actions ?? [];
  const nextAction = pendingActions.length > 0 ? pendingActions[0] : null;
  const processingRef = useRef(false);

  const [browserScreenshot] = browserApi.useBrowserScreenshotMutation();
  const [browserContext] = browserApi.useBrowserContextMutation();
  const [browserCurl] = browserApi.useBrowserCurlMutation();
  const [browserElementPick] = browserApi.useBrowserElementPickMutation();
  const [browserElementPickResult] =
    browserApi.useBrowserElementPickResultMutation();

  const executeAction = useCallback(
    async (action: string) => {
      if (!port) return;

      switch (action) {
        case "screenshot":
        case "screenshot_full": {
          const fullPage = action === "screenshot_full";
          const result = await browserScreenshot({
            chat_id: chatId,
            full_page: fullPage,
          }).unwrap();
          dispatch(
            addThreadImage({
              id: chatId,
              image: {
                name: fullPage ? "full_page.png" : "screenshot.png",
                content: `data:${result.mime};base64,${result.data}`,
                type: result.mime,
              },
            }),
          );
          break;
        }

        case "pick_element": {
          dispatch(setPickerActive({ chatId, active: true }));
          try {
            await browserElementPick({ chat_id: chatId }).unwrap();
            const pollInterval = 500;
            const maxAttempts = 60;
            for (let i = 0; i < maxAttempts; i++) {
              await new Promise((r) => setTimeout(r, pollInterval));
              const pickResult = await browserElementPickResult({
                chat_id: chatId,
              }).unwrap();
              if ("status" in pickResult) {
                continue;
              }
              if ("selector" in pickResult) {
                const text = `Selector: ${pickResult.selector}\nText: ${pickResult.innerText}\nBbox: ${JSON.stringify(pickResult.bbox)}`;
                await sendUserMessage(
                  chatId,
                  text,
                  port,
                  apiKey ?? undefined,
                );
              }
              break;
            }
          } finally {
            dispatch(setPickerActive({ chatId, active: false }));
          }
          break;
        }

        case "paste_actions":
        case "paste_console":
        case "paste_network": {
          const fieldMap = {
            paste_actions: "actions",
            paste_console: "console",
            paste_network: "network",
          } as const;
          const field = fieldMap[action as keyof typeof fieldMap];
          const result = await browserContext({
            chat_id: chatId,
            skip_cursor: true,
          }).unwrap();
          const content = JSON.stringify(
            result[field as keyof typeof result],
            null,
            2,
          );
          await sendUserMessage(chatId, content, port, apiKey ?? undefined);
          break;
        }

        case "curl": {
          const result = await browserCurl({ chat_id: chatId }).unwrap();
          await sendUserMessage(
            chatId,
            result.curl,
            port,
            apiKey ?? undefined,
          );
          break;
        }

        case "summarize":
        case "extract_json": {
          const result = await browserScreenshot({
            chat_id: chatId,
            full_page: false,
          }).unwrap();
          dispatch(
            updateBrowserFrame({
              chatId,
              frame: {
                mime: result.mime,
                data: result.data,
                diff_boxes: [],
              },
            }),
          );
          dispatch(
            addThreadImage({
              id: chatId,
              image: {
                name: "screenshot.png",
                content: `data:${result.mime};base64,${result.data}`,
                type: result.mime,
              },
            }),
          );
          const message =
            action === "summarize"
              ? "Summarize this page"
              : "Extract data as JSON from tables/lists";
          await sendUserMessage(chatId, message, port, apiKey ?? undefined);
          break;
        }

        default:
          break;
      }
    },
    [
      browserScreenshot,
      browserContext,
      browserCurl,
      browserElementPick,
      browserElementPickResult,
      chatId,
      dispatch,
      port,
      apiKey,
    ],
  );

  useEffect(() => {
    if (!nextAction || processingRef.current) return;
    processingRef.current = true;
    dispatch(shiftPendingToolbarAction({ chatId }));

    void executeAction(nextAction)
      .catch((err: unknown) => {
        // eslint-disable-next-line no-console
        console.warn("[BrowserToolbar] action failed:", nextAction, err);
      })
      .finally(() => {
        processingRef.current = false;
      });
  }, [nextAction, chatId, dispatch, executeAction]);
}
