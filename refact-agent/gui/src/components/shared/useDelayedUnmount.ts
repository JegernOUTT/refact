import { useState, useEffect, useLayoutEffect } from "react";

/**
 * Hook that handles mount/unmount with animations.
 * Returns { shouldRender, isAnimatingOpen } where:
 * - shouldRender: true while content should be in DOM (including during animations)
 * - isAnimatingOpen: true when the open animation should be applied (delayed by 1 frame on mount)
 *
 * @param isOpen - Whether the content should be visible
 * @param delayMs - How long to wait before unmounting (should match animation duration)
 * @param animate - Whether to animate the transition (when false, state changes are instant)
 */
export function useDelayedUnmount(
  isOpen: boolean,
  delayMs = 200,
  animate = true,
): { shouldRender: boolean; isAnimatingOpen: boolean } {
  const [shouldRender, setShouldRender] = useState(isOpen);
  const [isAnimatingOpen, setIsAnimatingOpen] = useState(isOpen);

  useEffect(() => {
    if (isOpen) {
      setShouldRender(true);
      if (!animate) {
        setIsAnimatingOpen(true);
      }
    } else {
      setIsAnimatingOpen(false);
      if (!animate) {
        setShouldRender(false);
        return;
      }
      const timer = setTimeout(() => {
        setShouldRender(false);
      }, delayMs);
      return () => clearTimeout(timer);
    }
  }, [isOpen, delayMs, animate]);

  useLayoutEffect(() => {
    if (isOpen && shouldRender && animate) {
      const raf = requestAnimationFrame(() => {
        setIsAnimatingOpen(true);
      });
      return () => cancelAnimationFrame(raf);
    }
  }, [isOpen, shouldRender, animate]);

  return { shouldRender, isAnimatingOpen };
}
