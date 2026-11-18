import { useCallback, useEffect, useRef, useState } from "react";
import {
  browserApi,
  projectBrowserErrorMessage,
  type ProjectBrowserFrame,
  type ProjectBrowserInputRequest,
  type ProjectBrowserStatus,
  type ProjectBrowserViewport,
} from "../../services/refact/browser";

export function useProjectBrowser() {
  const [getStatus] = browserApi.useProjectBrowserStatusMutation();
  const [start] = browserApi.useProjectBrowserStartMutation();
  const [getFrame] = browserApi.useProjectBrowserFrameMutation();
  const [resize] = browserApi.useProjectBrowserResizeMutation();
  const [input] = browserApi.useProjectBrowserInputMutation();
  const [status, setStatus] = useState<ProjectBrowserStatus>();
  const [frame, setFrame] = useState<ProjectBrowserFrame>();
  const [busy, setBusy] = useState(false);
  const [requestError, setRequestError] = useState<string>();
  const [refreshError, setRefreshError] = useState<string>();
  const content = useRef<HTMLDivElement>(null);
  const requestedSize = useRef<ProjectBrowserViewport>();
  const exclusive = useRef(false);
  const generation = useRef(0);
  const current = useRef<ProjectBrowserStatus>();
  const inputQueue = useRef(Promise.resolve());
  const pendingInputs = useRef(0);

  const publishStatus = useCallback((next: ProjectBrowserStatus) => {
    const previous = current.current;
    if (
      !previous ||
      next.runtime_id !== previous.runtime_id ||
      next.active_tab !== previous.active_tab
    )
      generation.current += 1;
    current.current = next;
    setStatus(next);
  }, []);

  const command = useCallback(
    async (run: () => Promise<ProjectBrowserStatus>) => {
      if (exclusive.current) return;
      exclusive.current = true;
      generation.current += 1;
      setBusy(true);
      setFrame(undefined);
      setRequestError(undefined);
      try {
        await inputQueue.current;
        publishStatus(await run());
      } catch (error) {
        setRequestError(projectBrowserErrorMessage(error));
      } finally {
        exclusive.current = false;
        setBusy(false);
      }
    },
    [publishStatus],
  );

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
    let mayStart = true;
    let lastResize: string | undefined;
    const poll = async () => {
      const epoch = generation.current;
      const valid = () =>
        !disposed && epoch === generation.current && !exclusive.current;
      if (!valid()) return;
      try {
        const snapshot = await getStatus(undefined).unwrap();
        if (!valid()) return;
        publishStatus(snapshot);
        const frameEpoch = generation.current;
        const validFrame = () =>
          !disposed && frameEpoch === generation.current && !exclusive.current;

        if (mayStart) {
          mayStart = false;
          if (
            !snapshot.running &&
            (snapshot.migration.state === "not_needed" ||
              snapshot.migration.state === "resolved")
          ) {
            await command(() => start(undefined).unwrap());
            return;
          }
        }
        if (!snapshot.connected || !snapshot.active_tab || snapshot.busy) {
          if (!snapshot.connected) setFrame(undefined);
          setRefreshError(undefined);
          return;
        }

        const size = requestedSize.current;
        if (size) {
          const resizeKey = `${snapshot.runtime_id}:${size.width}x${size.height}`;
          if (lastResize !== resizeKey) {
            await resize(size).unwrap();
            if (!validFrame()) return;
            lastResize = resizeKey;
          }
        }
        const next = await getFrame({}).unwrap();
        if (!validFrame()) return;
        if (
          next.runtime_id === snapshot.runtime_id &&
          next.tab_id === snapshot.active_tab
        ) {
          setFrame(next);
        }
        setRefreshError(undefined);
      } catch (error) {
        if (!disposed && !exclusive.current) {
          setFrame(undefined);
          setRefreshError(projectBrowserErrorMessage(error));
        }
      }
    };
    const tick = async () => {
      if (!document.hidden) await poll();
      if (!disposed) {
        timer = window.setTimeout(
          () => void tick(),
          current.current?.connected ? 350 : 1100,
        );
      }
    };
    void tick();
    return () => {
      disposed = true;
      window.clearTimeout(timer);
      generation.current += 1;
    };
  }, [command, getFrame, getStatus, publishStatus, resize, start]);

  const sendInput = useCallback(
    (request: ProjectBrowserInputRequest) => {
      if (exclusive.current || pendingInputs.current >= 32) return;
      const epoch = generation.current;
      pendingInputs.current += 1;
      inputQueue.current = inputQueue.current.then(async () => {
        try {
          const snapshot = current.current;
          if (
            epoch !== generation.current ||
            !snapshot?.connected ||
            snapshot.busy ||
            snapshot.runtime_id !== request.runtime_id ||
            snapshot.active_tab !== request.tab_id
          )
            return;
          await input(request).unwrap();
          if (epoch === generation.current) setRequestError(undefined);
        } catch (error) {
          if (epoch === generation.current) {
            setRequestError(projectBrowserErrorMessage(error));
            generation.current += 1;
            setFrame(undefined);
          }
        } finally {
          pendingInputs.current -= 1;
        }
      });
    },
    [input],
  );

  return {
    busy,
    command,
    content,
    error: requestError ?? refreshError,
    frame:
      status?.connected &&
      frame &&
      frame.runtime_id === status.runtime_id &&
      frame.tab_id === status.active_tab
        ? frame
        : undefined,
    sendInput,
    start,
    status,
  };
}
