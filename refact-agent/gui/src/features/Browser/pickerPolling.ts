import type { BrowserElementPickResultResponse } from "../../services/refact/browser";

export type BrowserPickedElement = {
  selector: string;
  innerText: string;
  bbox: { x: number; y: number; width: number; height: number };
};

export const PICKER_POLL_INTERVAL_MS = 500;
export const PICKER_POLL_MAX_ATTEMPTS = 60;

export type PickerPollingDeps = {
  poll: () => Promise<BrowserElementPickResultResponse>;
  cancel: () => Promise<unknown>;
  wait: (ms: number) => Promise<void>;
  pollIntervalMs?: number;
  maxAttempts?: number;
};

export async function runPickerPolling(
  deps: PickerPollingDeps,
): Promise<BrowserPickedElement | null> {
  const interval = deps.pollIntervalMs ?? PICKER_POLL_INTERVAL_MS;
  const attempts = deps.maxAttempts ?? PICKER_POLL_MAX_ATTEMPTS;
  let picked: BrowserPickedElement | null = null;

  try {
    for (let attempt = 0; attempt < attempts; attempt++) {
      await deps.wait(interval);
      const result = await deps.poll();
      if ("selector" in result) {
        picked = {
          selector: result.selector,
          innerText: result.innerText,
          bbox: result.bbox,
        };
        break;
      }
    }
  } finally {
    if (!picked) {
      await deps.cancel();
    }
  }

  return picked;
}
