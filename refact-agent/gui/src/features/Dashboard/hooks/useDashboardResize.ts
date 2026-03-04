import { useState, useCallback, useEffect, RefObject } from "react";

const MIN_RATIO = 0.2;
const MAX_RATIO = 0.8;

function clampRatio(n: number): number {
  if (!Number.isFinite(n)) return MIN_RATIO;
  return Math.max(MIN_RATIO, Math.min(MAX_RATIO, n));
}

function loadRatio(key: string, defaultValue: number): number {
  const safeDefault = clampRatio(defaultValue);
  try {
    const val = localStorage.getItem(key);
    if (!val) return safeDefault;
    const n = Number.parseFloat(val);
    if (!Number.isFinite(n)) return safeDefault;
    return clampRatio(n);
  } catch {
    return safeDefault;
  }
}

// Clean up old keys from previous layout iterations
function cleanupOldKeys() {
  try {
    localStorage.removeItem("dashboard:v1:top_ratio");
    localStorage.removeItem("dashboard:v1:bottom_ratio");
  } catch { /* ignore */ }
}

export function useDashboardResize(
  containerRef: RefObject<HTMLDivElement>,
  storageKey = "dashboard:v1:split_ratio",
  defaultRatio = 0.5,
) {
  const [ratio, setRatio] = useState<number>(() => loadRatio(storageKey, defaultRatio));

  useEffect(() => { cleanupOldKeys(); }, []);

  const handleDrag = useCallback((clientY: number) => {
    const container = containerRef.current;
    if (!container) return;
    const rect = container.getBoundingClientRect();
    if (!Number.isFinite(rect.height) || rect.height <= 0) return;
    const newRatio = clampRatio((clientY - rect.top) / rect.height);
    setRatio(newRatio);
    try { localStorage.setItem(storageKey, String(newRatio)); } catch { /* ignore */ }
  }, [containerRef, storageKey]);

  const reset = useCallback(() => {
    const safe = clampRatio(defaultRatio);
    setRatio(safe);
    try { localStorage.removeItem(storageKey); } catch { /* ignore */ }
  }, [defaultRatio, storageKey]);

  return { ratio, handleDrag, reset };
}
