import React from "react";
import {
  useChatScrollAnchor,
  usePrepareChatScrollAnchor,
} from "../useChatScrollAnchor";

export interface TranscriptAnchoredToggle {
  prepare: (event: React.SyntheticEvent) => void;
  run: (trigger: HTMLElement, mutate: () => void) => void;
}

export function useTranscriptAnchoredToggle(
  settleRef: React.RefObject<HTMLElement>,
): TranscriptAnchoredToggle {
  const preserveScrollAnchor = useChatScrollAnchor();
  const prepareScrollAnchor = usePrepareChatScrollAnchor();

  const prepare = React.useCallback(
    (event: React.SyntheticEvent) => {
      if (event.currentTarget instanceof HTMLElement) {
        prepareScrollAnchor(event.currentTarget);
      }
    },
    [prepareScrollAnchor],
  );

  const run = React.useCallback(
    (trigger: HTMLElement, mutate: () => void) => {
      preserveScrollAnchor(mutate, {
        trigger,
        settleElement: () => settleRef.current,
      });
    },
    [preserveScrollAnchor, settleRef],
  );

  return React.useMemo(() => ({ prepare, run }), [prepare, run]);
}
