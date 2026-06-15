import { useState, useEffect, useLayoutEffect } from "react";
import { useReducedMotion } from "../../hooks/useReducedMotion";

export const COLLAPSE_ANIMATION_MS = 200;

export function useDelayedUnmount(
  isOpen: boolean,
  delayMs = 150,
  animate = true,
): { shouldRender: boolean; isAnimatingOpen: boolean } {
  const reducedMotion = useReducedMotion();
  const shouldAnimate = animate && !reducedMotion;
  const [shouldRender, setShouldRender] = useState(isOpen);
  const [isAnimatingOpen, setIsAnimatingOpen] = useState(isOpen);

  useLayoutEffect(() => {
    if (!shouldAnimate) {
      setShouldRender(isOpen);
      setIsAnimatingOpen(isOpen);
      return;
    }

    if (isOpen) {
      setShouldRender(true);
      setIsAnimatingOpen(true);
      return;
    }

    setIsAnimatingOpen(false);
  }, [isOpen, shouldAnimate]);

  useEffect(() => {
    if (isOpen || !shouldRender) return;

    if (!shouldAnimate) {
      setShouldRender(false);
      return;
    }

    const timer = setTimeout(() => {
      setShouldRender(false);
    }, delayMs);
    return () => clearTimeout(timer);
  }, [isOpen, shouldRender, delayMs, shouldAnimate]);

  return { shouldRender, isAnimatingOpen };
}
