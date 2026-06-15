import { useSyncExternalStore } from "react";

const getMediaQuery = (query: string) => {
  if (
    typeof window === "undefined" ||
    typeof window.matchMedia !== "function"
  ) {
    return null;
  }

  return window.matchMedia(query);
};

export const useMediaQuery = (query: string) => {
  return useSyncExternalStore(
    (onStoreChange) => {
      const mediaQuery = getMediaQuery(query);

      if (!mediaQuery) {
        return () => undefined;
      }

      if (typeof mediaQuery.addEventListener === "function") {
        mediaQuery.addEventListener("change", onStoreChange);
        return () => mediaQuery.removeEventListener("change", onStoreChange);
      }

      mediaQuery.addListener(onStoreChange);
      return () => mediaQuery.removeListener(onStoreChange);
    },
    () => getMediaQuery(query)?.matches ?? false,
    () => false,
  );
};
