import { useState, useEffect, useLayoutEffect } from "react";
import { useReducedMotion } from "../../hooks/useReducedMotion";

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
      if (!shouldRender) {
        setIsAnimatingOpen(true);
      }
      return;
    }

    setIsAnimatingOpen(false);
  }, [isOpen, shouldRender, shouldAnimate]);

  useLayoutEffect(() => {
    if (!isOpen || !shouldRender || !shouldAnimate || isAnimatingOpen) return;

    const open = () => setIsAnimatingOpen(true);
    const raf =
      typeof requestAnimationFrame === "function"
        ? requestAnimationFrame(open)
        : null;
    const timer = setTimeout(open, 0);

    return () => {
      if (raf !== null && typeof cancelAnimationFrame === "function") {
        cancelAnimationFrame(raf);
      }
      clearTimeout(timer);
    };
  }, [isOpen, shouldRender, shouldAnimate, isAnimatingOpen]);

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
