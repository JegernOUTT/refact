import { useCallback, useLayoutEffect, useRef, useState } from "react";
import { useIsDocumentVisible } from "../../hooks/useIsDocumentVisible";
import {
  browserApi,
  isProjectBrowserTabGone,
  projectBrowserErrorMessage,
  type ProjectBrowserInputFence,
  type ProjectBrowserInputRequest,
  type ProjectBrowserStatus,
} from "../../services/refact/browser";
import { useBrowserFrameStream } from "./useBrowserFrameStream";
import { useBrowserViewport } from "./useBrowserViewport";

const MAX_PENDING_INPUTS = 32;

const hasTab = (status: ProjectBrowserStatus | undefined, tabId: string) =>
  Boolean(status?.tabs.some((tab) => tab.tab_id === tabId));

export function useBrowserTabFrame(
  tabId: string,
  status: ProjectBrowserStatus | undefined,
  active = true,
  awaitingStatus = false,
  navigating = false,
) {
  const visible = useIsDocumentVisible();
  const [input] = browserApi.useProjectBrowserInputMutation();
  const [closedTarget, setClosedTarget] = useState<ProjectBrowserInputFence>();
  const [inputError, setInputError] = useState<string>();
  const statusRef = useRef(status);
  statusRef.current = status;
  const generation = useRef(0);
  const inputQueue = useRef(Promise.resolve());
  const pendingInputs = useRef(0);
  const runtimeId = status?.runtime_id;
  const inStatus = hasTab(status, tabId);
  const tabListed = awaitingStatus
    ? undefined
    : status && !status.running
      ? false
      : status?.connected && !status.busy
        ? inStatus
        : undefined;
  const closed =
    tabListed === false ||
    (closedTarget?.tab_id === tabId && closedTarget.runtime_id === runtimeId);
  const streamEnabled =
    active &&
    visible &&
    !closed &&
    !awaitingStatus &&
    status?.connected === true &&
    Boolean(runtimeId) &&
    inStatus;
  const inputEnabled = streamEnabled && !status.busy && !navigating;
  const markClosed = useCallback(() => {
    generation.current += 1;
    if (runtimeId) setClosedTarget({ runtime_id: runtimeId, tab_id: tabId });
  }, [runtimeId, tabId]);

  const { content, error: resizeError } = useBrowserViewport(
    streamEnabled,
    runtimeId,
    tabId,
    markClosed,
  );
  const { frame, error: streamError } = useBrowserFrameStream(
    status?.connected && !closed ? runtimeId : undefined,
    tabId,
    streamEnabled,
    markClosed,
  );

  useLayoutEffect(() => {
    setInputError(undefined);
    return () => {
      generation.current += 1;
    };
  }, [runtimeId, tabId]);

  const sendInput = useCallback(
    (request: ProjectBrowserInputRequest) => {
      if (!inputEnabled || pendingInputs.current >= MAX_PENDING_INPUTS) return;
      const epoch = generation.current;
      pendingInputs.current += 1;
      inputQueue.current = inputQueue.current.then(async () => {
        try {
          const snapshot = statusRef.current;
          if (
            epoch !== generation.current ||
            !snapshot?.connected ||
            snapshot.busy ||
            snapshot.runtime_id !== request.runtime_id ||
            request.tab_id !== tabId ||
            !hasTab(snapshot, tabId)
          )
            return;
          await input(request).unwrap();
          if (epoch === generation.current) setInputError(undefined);
        } catch (error) {
          if (epoch === generation.current) {
            generation.current += 1;
            if (isProjectBrowserTabGone(error)) markClosed();
            else setInputError(projectBrowserErrorMessage(error));
          }
        } finally {
          pendingInputs.current -= 1;
        }
      });
    },
    [inputEnabled, input, tabId, markClosed],
  );

  const invalidate = useCallback(async () => {
    generation.current += 1;
    setInputError(undefined);
    await inputQueue.current;
  }, []);

  return {
    content,
    frame:
      status?.connected &&
      !closed &&
      frame?.runtime_id === runtimeId &&
      frame?.tab_id === tabId
        ? frame
        : undefined,
    closed,
    error: inputError ?? streamError ?? resizeError,
    sendInput,
    invalidate,
    markClosed,
  };
}
