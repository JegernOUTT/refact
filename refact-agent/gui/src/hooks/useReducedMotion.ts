import { useSyncExternalStore } from "react";

const REDUCED_MOTION_QUERY = "(prefers-reduced-motion: reduce)";

const getMediaQuery = () => {
  if (
    typeof window === "undefined" ||
    typeof window.matchMedia !== "function"
  ) {
    return null;
  }

  return window.matchMedia(REDUCED_MOTION_QUERY);
};

const subscribe = (onStoreChange: () => void) => {
  const mediaQuery = getMediaQuery();

  if (!mediaQuery) {
    return () => undefined;
  }

  if (typeof mediaQuery.addEventListener === "function") {
    mediaQuery.addEventListener("change", onStoreChange);
    return () => mediaQuery.removeEventListener("change", onStoreChange);
  }

  mediaQuery.addListener(onStoreChange);
  return () => mediaQuery.removeListener(onStoreChange);
};

const getSnapshot = () => getMediaQuery()?.matches ?? false;

export const useReducedMotion = () => {
  return useSyncExternalStore(subscribe, getSnapshot, () => false);
};
