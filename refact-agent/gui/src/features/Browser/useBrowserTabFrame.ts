import { useCallback, useEffect, useRef, useState } from "react";
import {
  browserApi,
  isProjectBrowserConflict,
  isProjectBrowserTabGone,
  projectBrowserErrorMessage,
  type ProjectBrowserFrame,
  type ProjectBrowserInputRequest,
  type ProjectBrowserStatus,
  type ProjectBrowserViewport,
} from "../../services/refact/browser";

export const BROWSER_FRAME_POLL_MS = 350;
const BROWSER_IDLE_POLL_MS = 1100;
const MAX_PENDING_INPUTS = 32;

const hasTab = (status: ProjectBrowserStatus | undefined, tabId: string) =>
  Boolean(status?.tabs.some((tab) => tab.tab_id === tabId));

export function useBrowserTabFrame(
  tabId: string,
  status: ProjectBrowserStatus | undefined,
  active = true,
  awaitingStatus = false,
) {
  const [getFrame] = browserApi.useProjectBrowserFrameMutation();
  const [resize] = browserApi.useProjectBrowserResizeMutation();
  const [input] = browserApi.useProjectBrowserInputMutation();
  const [frame, setFrame] = useState<ProjectBrowserFrame>();
  const [closedByEngine, setClosedByEngine] = useState(false);
  const [refreshError, setRefreshError] = useState<string>();
  const [inputError, setInputError] = useState<string>();
  const content = useRef<HTMLDivElement>(null);
  const requestedSize = useRef<ProjectBrowserViewport>();
  const statusRef = useRef(status);
  statusRef.current = status;
  const activeRef = useRef(active);
  activeRef.current = active;
  const generation = useRef(0);
  const inputQueue = useRef(Promise.resolve());
  const pendingInputs = useRef(0);

  useEffect(() => {
    const element = content.current;
    if (!element) return;
    const measure = (width: number, height: number) => {
      requestedSize.current =
        width > 0 && height > 0
          ? {
              width: Math.min(4096, Math.max(120, Math.round(width))),
              height: Math.min(4096, Math.max(120, Math.round(height))),
            }
          : undefined;
    };
    measure(element.clientWidth, element.clientHeight);
    const observer = new ResizeObserver((entries) => {
      for (const { contentRect } of entries)
        measure(contentRect.width, contentRect.height);
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    let disposed = false;
    let timer: number | undefined;
    let lastResize: string | undefined;
    generation.current += 1;
    setFrame(undefined);
    setClosedByEngine(false);
    const poll = async () => {
      const snapshot = statusRef.current;
      if (
        !snapshot?.connected ||
        !snapshot.runtime_id ||
        snapshot.busy ||
        !hasTab(snapshot, tabId)
      )
        return;
      const runtimeId = snapshot.runtime_id;
      const epoch = generation.current;
      const valid = () => !disposed && epoch === generation.current;
      try {
        const size = requestedSize.current;
        if (size) {
          const resizeKey = `${runtimeId}:${size.width}x${size.height}`;
          if (lastResize !== resizeKey) {
            await resize({ ...size, tab_id: tabId }).unwrap();
            if (!valid()) return;
            lastResize = resizeKey;
          }
        }
        const next = await getFrame({ tab_id: tabId }).unwrap();
        if (!valid()) return;
        if (
          next.tab_id === tabId &&
          next.runtime_id === statusRef.current?.runtime_id
        ) {
          setFrame(next);
          setClosedByEngine(false);
        }
        setRefreshError(undefined);
      } catch (error) {
        if (!valid()) return;
        if (isProjectBrowserTabGone(error)) {
          setFrame(undefined);
          setClosedByEngine(true);
          setRefreshError(undefined);
        } else if (isProjectBrowserConflict(error)) {
          setRefreshError(undefined);
        } else {
          setFrame(undefined);
          setRefreshError(projectBrowserErrorMessage(error));
        }
      }
    };
    const tick = async () => {
      if (!document.hidden && activeRef.current) await poll();
      if (!disposed)
        timer = window.setTimeout(
          () => void tick(),
          statusRef.current?.connected === false
            ? BROWSER_IDLE_POLL_MS
            : BROWSER_FRAME_POLL_MS,
        );
    };
    void tick();
    return () => {
      disposed = true;
      window.clearTimeout(timer);
      generation.current += 1;
    };
  }, [getFrame, resize, tabId]);

  const sendInput = useCallback(
    (request: ProjectBrowserInputRequest) => {
      if (pendingInputs.current >= MAX_PENDING_INPUTS) return;
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
            setFrame(undefined);
            if (isProjectBrowserTabGone(error)) setClosedByEngine(true);
            else setInputError(projectBrowserErrorMessage(error));
          }
        } finally {
          pendingInputs.current -= 1;
        }
      });
    },
    [input, tabId],
  );

  const invalidate = useCallback(async () => {
    generation.current += 1;
    setFrame(undefined);
    setInputError(undefined);
    await inputQueue.current;
  }, []);

  const markClosed = useCallback(() => {
    generation.current += 1;
    setFrame(undefined);
    setClosedByEngine(true);
  }, []);

  const tabListed = awaitingStatus
    ? undefined
    : status && !status.running
      ? false
      : status?.connected && !status.busy
        ? hasTab(status, tabId)
        : undefined;
  const liveFrame =
    status?.connected &&
    frame &&
    frame.runtime_id === status.runtime_id &&
    frame.tab_id === tabId
      ? frame
      : undefined;

  return {
    content,
    frame: liveFrame,
    closed: tabListed === false || (closedByEngine && !liveFrame),
    error: inputError ?? refreshError,
    sendInput,
    invalidate,
    markClosed,
  };
}
