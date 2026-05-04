import { useDeferredValue, useEffect, useRef, useState } from "react";

const STREAMING_MARKDOWN_UPDATE_MS = 150;

export function useStreamingMarkdown(
  text: string | null,
  isStreaming: boolean,
): string | null {
  const deferredText = useDeferredValue(text);
  const [mountedText, setMountedText] = useState<string | null>(
    isStreaming ? deferredText : text,
  );
  const latestTextRef = useRef<string | null>(
    isStreaming ? deferredText : text,
  );
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    latestTextRef.current = isStreaming ? deferredText : text;

    if (!isStreaming) {
      if (timerRef.current !== null) {
        clearTimeout(timerRef.current);
        timerRef.current = null;
      }
      setMountedText(text);
      return;
    }

    if (timerRef.current !== null) return;

    timerRef.current = setTimeout(() => {
      timerRef.current = null;
      setMountedText(latestTextRef.current);
    }, STREAMING_MARKDOWN_UPDATE_MS);
  }, [deferredText, isStreaming, text]);

  useEffect(() => {
    return () => {
      if (timerRef.current !== null) {
        clearTimeout(timerRef.current);
        timerRef.current = null;
      }
    };
  }, []);

  return isStreaming ? mountedText : text;
}
