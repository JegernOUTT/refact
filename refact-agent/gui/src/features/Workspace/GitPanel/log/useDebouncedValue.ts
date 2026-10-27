import { useEffect, useRef, useState } from "react";

export const FILTER_DEBOUNCE_MS = 300;

/** Local input state that reports to `onCommit` after typing pauses. */
export function useDebouncedInput(
  value: string,
  onCommit: (value: string) => void,
  delay = FILTER_DEBOUNCE_MS,
) {
  const [draft, setDraft] = useState(value);
  const commitRef = useRef(onCommit);
  commitRef.current = onCommit;
  const lastCommitted = useRef(value);

  useEffect(() => {
    if (value === lastCommitted.current) return;
    lastCommitted.current = value;
    setDraft(value);
  }, [value]);

  useEffect(() => {
    if (draft === lastCommitted.current) return;
    const timer = window.setTimeout(() => {
      lastCommitted.current = draft;
      commitRef.current(draft);
    }, delay);
    return () => window.clearTimeout(timer);
  }, [delay, draft]);

  return [draft, setDraft] as const;
}
