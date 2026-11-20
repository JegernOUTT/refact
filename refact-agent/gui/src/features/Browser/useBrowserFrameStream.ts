import { useCallback, useLayoutEffect, useRef, useState } from "react";
import { useAppSelector } from "../../hooks";
import { buildApiUrlFromState } from "../../services/refact/apiUrl";
import {
  BROWSER_PROJECT_FRAMES_SUBSCRIBE,
  isProjectBrowserConflict,
  isProjectBrowserTabGone,
} from "../../services/refact/browser";
import {
  subscribeProjectBrowserFrames,
  type BrowserFrameHeader,
  type ReceivedBrowserFrame,
} from "../../services/refact/projectBrowserStream";

export type DisplayedBrowserFrame = BrowserFrameHeader & { src: string };

export function useBrowserFrameStream(
  runtimeId: string | null | undefined,
  tabId: string,
  enabled: boolean,
  onGone: () => void,
) {
  const url = useAppSelector((state) =>
    buildApiUrlFromState(state, BROWSER_PROJECT_FRAMES_SUBSCRIBE),
  );
  const apiKey = useAppSelector((state) => state.config.apiKey);
  const [frame, setFrame] = useState<DisplayedBrowserFrame>();
  const [error, setError] = useState<string>();
  const published = useRef<string>();

  const clear = useCallback(() => {
    if (published.current) URL.revokeObjectURL(published.current);
    published.current = undefined;
    setFrame(undefined);
  }, []);

  useLayoutEffect(() => {
    clear();
    setError(undefined);
    return () => {
      if (published.current) URL.revokeObjectURL(published.current);
      published.current = undefined;
    };
  }, [clear, url, apiKey, runtimeId, tabId]);

  useLayoutEffect(() => {
    if (!enabled || !runtimeId) return;
    const lifetime = new AbortController();
    const stopped = () => lifetime.signal.aborted;
    let timer: number | undefined;
    let animation: number | undefined;
    let latest: ReceivedBrowserFrame | undefined;
    let decoding: string | undefined;
    let retryMs = 500;

    const scheduleDecode = () => {
      if (
        animation === undefined &&
        !decoding &&
        latest &&
        !lifetime.signal.aborted
      ) {
        animation = requestAnimationFrame(() => void decodeLatest());
      }
    };
    const decodeLatest = async () => {
      animation = undefined;
      if (!latest || lifetime.signal.aborted) return;
      const { bytes, ...metadata } = latest;
      latest = undefined;
      const image = new Image();
      try {
        decoding = URL.createObjectURL(
          new Blob([bytes], { type: metadata.mime }),
        );
        image.src = decoding;
        await image.decode();
        if (stopped()) return;
        if (
          image.naturalWidth !== metadata.width ||
          image.naturalHeight !== metadata.height
        )
          throw new Error("Frame dimensions do not match the decoded image");
        const previous = published.current;
        published.current = decoding;
        setFrame({ ...metadata, src: decoding });
        decoding = undefined;
        if (previous) URL.revokeObjectURL(previous);
      } catch {
        if (!stopped()) setError("Could not decode browser frame");
      } finally {
        if (decoding) URL.revokeObjectURL(decoding);
        decoding = undefined;
        scheduleDecode();
      }
    };
    const connect = async () => {
      try {
        await subscribeProjectBrowserFrames(
          url,
          apiKey,
          { runtime_id: runtimeId, tab_id: tabId },
          lifetime.signal,
          (received) => {
            latest = received;
            retryMs = 500;
            setError(undefined);
            scheduleDecode();
          },
        );
      } catch (cause) {
        if (stopped()) return;
        if (isProjectBrowserTabGone(cause)) {
          onGone();
          return;
        }
        setError(
          isProjectBrowserConflict(cause)
            ? undefined
            : cause instanceof Error
              ? cause.message
              : "Browser frame stream failed",
        );
        timer = window.setTimeout(() => void connect(), retryMs);
        retryMs = Math.min(8000, retryMs * 2);
      }
    };
    void connect();
    return () => {
      lifetime.abort();
      window.clearTimeout(timer);
      if (animation !== undefined) cancelAnimationFrame(animation);
      if (decoding) URL.revokeObjectURL(decoding);
      decoding = undefined;
      latest = undefined;
    };
  }, [enabled, url, apiKey, runtimeId, tabId, onGone]);

  return { frame, error };
}
