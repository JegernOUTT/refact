import { useEffect, useRef, useState } from "react";

function isDocumentVisible(): boolean {
  return (
    typeof document === "undefined" || document.visibilityState !== "hidden"
  );
}

export function useIsDocumentVisible(onVisible?: () => void): boolean {
  const [visible, setVisible] = useState(isDocumentVisible);
  const wasVisible = useRef(visible);
  const onVisibleRef = useRef(onVisible);
  onVisibleRef.current = onVisible;

  useEffect(() => {
    const onVisibilityChange = () => {
      const nextVisible = isDocumentVisible();
      const resumed = nextVisible && !wasVisible.current;
      wasVisible.current = nextVisible;
      setVisible(nextVisible);
      if (resumed) onVisibleRef.current?.();
    };
    document.addEventListener("visibilitychange", onVisibilityChange);
    return () =>
      document.removeEventListener("visibilitychange", onVisibilityChange);
  }, []);

  return visible;
}
