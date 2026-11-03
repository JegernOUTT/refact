import { useEffect, useState } from "react";
import { debugRefact } from "../../../debugConfig";
import { calculateBackoff } from "../../../utils/backoff";
import {
  hasUsableEngineEndpoint,
  type EngineApiConfig,
} from "../../../services/refact/apiUrl";
import {
  BuddyRequestError,
  getAttention,
  type AttentionRow,
} from "../../../services/refact/buddyCompanion";

export const ATTENTION_POLL_MS = 5_000;
export const ATTENTION_MAX_BACKOFF_MS = 60_000;

export type BuddyAttention = { rows: AttentionRow[]; count: number };

const empty: AttentionRow[] = [];

export function useBuddyAttention(
  config: EngineApiConfig & { apiKey?: string | null },
  namespace: string,
): BuddyAttention {
  const [rows, setRows] = useState<AttentionRow[]>(empty);
  const usable = hasUsableEngineEndpoint(config);

  useEffect(() => {
    setRows(empty);
    if (!usable) return;
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout> | undefined;
    let failures = 0;
    let stopped = false;
    let running = false;

    const schedule = (delay: number) => {
      clearTimeout(timer);
      if (stopped || controller.signal.aborted) return;
      timer = setTimeout(() => void poll(), delay);
    };

    const poll = async () => {
      if (running || stopped || controller.signal.aborted) return;
      if (document.visibilityState !== "visible") return;
      running = true;
      try {
        const snapshot = await getAttention(config, controller.signal);
        if (controller.signal.aborted) return;
        if (snapshot.schema_version !== 1)
          throw new Error("Unsupported Buddy attention schema");
        failures = 0;
        setRows(snapshot.rows);
        schedule(ATTENTION_POLL_MS);
      } catch (error) {
        if (controller.signal.aborted) return;
        if (error instanceof BuddyRequestError && error.status === 404) {
          stopped = true;
          debugRefact("Buddy attention is unavailable on this engine");
          return;
        }
        failures += 1;
        schedule(
          calculateBackoff(failures, {
            baseDelay: ATTENTION_POLL_MS,
            maxDelay: ATTENTION_MAX_BACKOFF_MS,
          }),
        );
      } finally {
        running = false;
      }
    };

    const onVisibility = () => {
      if (document.visibilityState === "visible") schedule(0);
      else clearTimeout(timer);
    };

    document.addEventListener("visibilitychange", onVisibility);
    void poll();
    return () => {
      stopped = true;
      controller.abort();
      clearTimeout(timer);
      document.removeEventListener("visibilitychange", onVisibility);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- namespace identifies the endpoint and credentials.
  }, [namespace, usable]);

  return { rows, count: rows.length };
}
