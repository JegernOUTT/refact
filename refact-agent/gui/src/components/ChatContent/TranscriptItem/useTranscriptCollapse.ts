import React from "react";
import { useReducedMotion } from "../../../hooks/useReducedMotion";
import { COLLAPSE_ANIMATION_MS } from "../../shared/useDelayedUnmount";

export const TRANSCRIPT_COLLAPSE_FALLBACK_MS = COLLAPSE_ANIMATION_MS;

function isReducedMotionForced(): boolean {
  if (typeof document === "undefined") return false;
  return document.documentElement.dataset.reducedMotion === "on";
}

export interface TranscriptCollapseState {
  mounted: boolean;
  expanded: boolean;
  panelRef: React.MutableRefObject<HTMLDivElement | null>;
  onTransitionEnd: (event: React.TransitionEvent<HTMLDivElement>) => void;
}

export function useTranscriptCollapse(
  open: boolean,
  animate = true,
): TranscriptCollapseState {
  const reducedMotion = useReducedMotion();
  const shouldAnimate = animate && !reducedMotion && !isReducedMotionForced();
  const panelRef = React.useRef<HTMLDivElement | null>(null);
  const [mounted, setMounted] = React.useState(open);
  const [expanded, setExpanded] = React.useState(open);

  React.useLayoutEffect(() => {
    if (!shouldAnimate) {
      setMounted(open);
      setExpanded(open);
      return;
    }
    if (open) setMounted(true);
    else setExpanded(false);
  }, [open, shouldAnimate]);

  React.useLayoutEffect(() => {
    if (!open || !mounted || expanded) return;
    void panelRef.current?.offsetHeight;
    setExpanded(true);
  }, [open, mounted, expanded]);

  React.useEffect(() => {
    if (open || !mounted || expanded) return;
    const timer = window.setTimeout(
      () => setMounted(false),
      TRANSCRIPT_COLLAPSE_FALLBACK_MS,
    );
    return () => window.clearTimeout(timer);
  }, [open, mounted, expanded]);

  const onTransitionEnd = React.useCallback(
    (event: React.TransitionEvent<HTMLDivElement>) => {
      if (event.target !== panelRef.current) return;
      if (!open) setMounted(false);
    },
    [open],
  );

  return { mounted, expanded, panelRef, onTransitionEnd };
}
