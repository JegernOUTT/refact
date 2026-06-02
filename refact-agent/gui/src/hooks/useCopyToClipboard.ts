import { useCallback } from "react";
import { fallbackCopying } from "../utils/fallbackCopying";

export const useCopyToClipboard = () => {
  return useCallback((text: string) => {
    const clipboard = (window.navigator as unknown as { clipboard?: Clipboard })
      .clipboard;
    if (!clipboard?.writeText) {
      fallbackCopying(text);
      return;
    }

    void clipboard.writeText(text).catch(() => {
      fallbackCopying(text);
    });
  }, []);
};
