import { useState, useEffect, useLayoutEffect } from "react";

/**
 * Hook that handles mount/unmount with animations.
 * Returns { shouldRender, isAnimating } where:
 * - shouldRender: true while content should be in DOM (including during animations)
 * - isAnimating: true when the open animation should be applied (delayed by 1 frame on mount)
 *
 * @param isOpen - Whether the content should be visible
 * @param delayMs - How long to wait before unmounting (should match animation duration)
 */
export function useDelayedUnmount(
  isOpen: boolean,
  delayMs = 200,
): { shouldRender: boolean; isAnimatingOpen: boolean } {
  const [shouldRender, setShouldRender] = useState(isOpen);
  const [isAnimatingOpen, setIsAnimatingOpen] = useState(isOpen);

  useEffect(() => {
    if (isOpen) {
      // Immediately mount when opening
      setShouldRender(true);
    } else {
      // Immediately start close animation
      setIsAnimatingOpen(false);
      // Delay unmount when closing to allow exit animation
      const timer = setTimeout(() => {
        setShouldRender(false);
      }, delayMs);
      return () => clearTimeout(timer);
    }
  }, [isOpen, delayMs]);

  // Use layoutEffect to trigger open animation after mount (next frame)
  useLayoutEffect(() => {
    if (isOpen && shouldRender) {
      // Request animation frame to ensure DOM has rendered in closed state first
      const raf = requestAnimationFrame(() => {
        setIsAnimatingOpen(true);
      });
      return () => cancelAnimationFrame(raf);
    }
  }, [isOpen, shouldRender]);

  return { shouldRender, isAnimatingOpen };
}
