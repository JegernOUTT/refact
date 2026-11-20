import { useEffect, useRef, useState } from "react";
import {
  browserApi,
  isProjectBrowserConflict,
  isProjectBrowserTabGone,
  projectBrowserErrorMessage,
  type ProjectBrowserViewport,
} from "../../services/refact/browser";

export function useBrowserViewport(
  enabled: boolean,
  runtimeId: string | null | undefined,
  tabId: string,
  onGone: () => void,
) {
  const content = useRef<HTMLDivElement>(null);
  const [resize] = browserApi.useProjectBrowserResizeMutation();
  const [error, setError] = useState<string>();
  useEffect(() => {
    const element = content.current;
    setError(undefined);
    if (!element || !enabled) return;
    let disposed = false;
    const isDisposed = () => disposed;
    let timer: number | undefined;
    let desired: ProjectBrowserViewport | undefined;
    let applied: ProjectBrowserViewport | undefined;
    let pending: ReturnType<typeof resize> | undefined;
    let retryMs = 500;
    const flush = async () => {
      const size = desired;
      if (
        isDisposed() ||
        size === undefined ||
        pending !== undefined ||
        size === applied
      )
        return;
      pending = resize({ ...size, tab_id: tabId });
      try {
        await pending.unwrap();
      } catch (cause) {
        if (isDisposed()) return;
        if (isProjectBrowserTabGone(cause)) onGone();
        else {
          if (!isProjectBrowserConflict(cause))
            setError(projectBrowserErrorMessage(cause));
          timer = window.setTimeout(() => void flush(), retryMs);
          retryMs = Math.min(8000, retryMs * 2);
        }
        return;
      } finally {
        pending = undefined;
      }
      if (isDisposed()) return;
      applied = size;
      setError(undefined);
      retryMs = 500;
      if (desired !== applied) void flush();
    };
    const measure = (w: number, h: number) => {
      if (w <= 0 || h <= 0) {
        desired = undefined;
        return;
      }
      const width = Math.min(4096, Math.max(120, Math.round(w)));
      const height = Math.min(4096, Math.max(120, Math.round(h)));
      if (desired?.width === width && desired.height === height) return;
      desired = { width, height };
      window.clearTimeout(timer);
      void flush();
    };
    measure(element.clientWidth, element.clientHeight);
    const observer = new ResizeObserver((entries) => {
      for (const { contentRect } of entries)
        measure(contentRect.width, contentRect.height);
    });
    observer.observe(element);
    return () => {
      disposed = true;
      observer.disconnect();
      window.clearTimeout(timer);
      pending?.abort();
    };
  }, [enabled, runtimeId, resize, tabId, onGone]);
  return { content, error };
}
