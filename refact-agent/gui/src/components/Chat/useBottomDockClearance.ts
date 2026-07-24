import { useLayoutEffect, type RefObject } from "react";

export function useBottomDockClearance(
  bottomDockRef: RefObject<HTMLElement>,
): void {
  useLayoutEffect(() => {
    const dock = bottomDockRef.current;
    const root = dock?.parentElement;
    if (!dock || !root) return;

    const updateClearance = () => {
      root.style.setProperty(
        "--rf-composer-clearance",
        `${dock.offsetHeight}px`,
      );
    };

    updateClearance();
    const observer = new ResizeObserver(updateClearance);
    observer.observe(dock);
    return () => observer.disconnect();
  }, [bottomDockRef]);
}
