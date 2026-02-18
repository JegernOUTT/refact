import { useEffect, useCallback, useRef } from "react";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { browserApi } from "../../services/refact/browser";
import type { BrowserAnnotation } from "../../services/refact/browser";
import {
  selectBrowserRuntime,
  shiftPendingToolbarAction,
  updateBrowserFrame,
  setPickerActive,
  setAnnotateActive,
  setBrowserNotification,
} from "./browserSlice";
import { addThreadImage } from "../Chat/Thread/actions";
import { formatBrowserDraftBlock, insertBrowserDraft } from "./draftInsert";

function formatAnnotationsText(annotations: BrowserAnnotation[]): string {
  if (annotations.length === 0) return "(no annotations)";
  return annotations
    .map(
      (a) =>
        `[${a.index}] ${a.selector} — "${a.innerText.substring(0, 120)}" (${
          a.bbox.x
        },${a.bbox.y} ${a.bbox.width}×${a.bbox.height})`,
    )
    .join("\n");
}

export function useBrowserToolbarActions(chatId: string) {
  const dispatch = useAppDispatch();
  const runtime = useAppSelector((state) =>
    selectBrowserRuntime(state, chatId),
  );
  const pendingActions = runtime?.pending_toolbar_actions ?? [];
  const nextAction = pendingActions.length > 0 ? pendingActions[0] : null;
  const processingRef = useRef(false);

  const [browserScreenshot] = browserApi.useBrowserScreenshotMutation();
  const [browserContext] = browserApi.useBrowserContextMutation();
  const [browserCurl] = browserApi.useBrowserCurlMutation();
  const [browserElementPick] = browserApi.useBrowserElementPickMutation();
  const [browserElementPickResult] =
    browserApi.useBrowserElementPickResultMutation();
  const [browserAnnotateStart] = browserApi.useBrowserAnnotateStartMutation();
  const [browserAnnotateResult] = browserApi.useBrowserAnnotateResultMutation();
  const [browserAnnotateClear] = browserApi.useBrowserAnnotateClearMutation();

  const notifyError = useCallback(
    (action: string, err: unknown) => {
      const message = err instanceof Error ? err.message : String(err);
      dispatch(
        setBrowserNotification({
          chatId,
          notification: {
            type: "timeout",
            message: `Action "${action}" failed: ${message}`,
          },
        }),
      );
    },
    [chatId, dispatch],
  );

  const executeAction = useCallback(
    async (action: string) => {
      switch (action) {
        case "annotate": {
          await browserAnnotateStart({ chat_id: chatId }).unwrap();
          dispatch(setAnnotateActive({ chatId, active: true }));
          break;
        }
        case "annotate_send": {
          try {
            const annotResult = await browserAnnotateResult({
              chat_id: chatId,
            }).unwrap();
            const screenshotResult = await browserScreenshot({
              chat_id: chatId,
              full_page: false,
            }).unwrap();
            const ext = screenshotResult.mime === "image/png" ? "png" : "jpg";
            dispatch(
              addThreadImage({
                id: chatId,
                image: {
                  name: `annotated_screenshot.${ext}`,
                  content: `data:${screenshotResult.mime};base64,${screenshotResult.data}`,
                  type: screenshotResult.mime,
                },
              }),
            );
            const annotText = formatAnnotationsText(annotResult.annotations);
            insertBrowserDraft(
              formatBrowserDraftBlock(
                "Browser Annotations",
                `The screenshot has numbered annotations:\n${annotText}`,
              ),
            );
            await browserAnnotateClear({ chat_id: chatId })
              .unwrap()
              .catch((_: unknown) => {
                /* best-effort cleanup */
              });
          } finally {
            dispatch(setAnnotateActive({ chatId, active: false }));
          }
          break;
        }
        case "annotate_clear": {
          try {
            await browserAnnotateClear({ chat_id: chatId }).unwrap();
          } finally {
            dispatch(setAnnotateActive({ chatId, active: false }));
          }
          break;
        }

        case "screenshot":
        case "screenshot_full": {
          const fullPage = action === "screenshot_full";
          const result = await browserScreenshot({
            chat_id: chatId,
            full_page: fullPage,
          }).unwrap();
          const ext = result.mime === "image/png" ? "png" : "jpg";
          dispatch(
            addThreadImage({
              id: chatId,
              image: {
                name: fullPage ? `full_page.${ext}` : `screenshot.${ext}`,
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
                const text = `Selector: ${pickResult.selector}\nText: ${
                  pickResult.innerText
                }\nBbox: ${JSON.stringify(pickResult.bbox)}`;
                insertBrowserDraft(
                  formatBrowserDraftBlock("Browser Picked Element", text),
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
          const title =
            action === "paste_actions"
              ? "Browser Actions"
              : action === "paste_console"
                ? "Browser Console"
                : "Browser Network";
          insertBrowserDraft(formatBrowserDraftBlock(title, content));
          break;
        }

        case "curl": {
          const result = await browserCurl({ chat_id: chatId }).unwrap();
          insertBrowserDraft(
            formatBrowserDraftBlock("Browser cURL", result.curl),
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
          insertBrowserDraft(formatBrowserDraftBlock("Browser Task", message));
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
      browserAnnotateStart,
      browserAnnotateResult,
      browserAnnotateClear,
      chatId,
      dispatch,
    ],
  );

  useEffect(() => {
    if (!nextAction || processingRef.current) return;
    processingRef.current = true;
    dispatch(shiftPendingToolbarAction({ chatId }));

    void executeAction(nextAction)
      .catch((err: unknown) => {
        notifyError(nextAction, err);
      })
      .finally(() => {
        processingRef.current = false;
      });
  }, [nextAction, chatId, dispatch, executeAction, notifyError]);
}
