import {
  causalSequence,
  parseCausalFrame,
  type CausalPage,
} from "./causalTypes";
import { subscribeToSnapshots } from "./snapshotStream";

export type CausalStreamOptions = {
  maxBytes: number;
  timeoutMs: number;
  onSnapshot: (page: CausalPage) => void;
  onError: (reason: string) => void;
};

export function subscribeToCausalTrace(
  url: string,
  token: string | null,
  options: CausalStreamOptions,
): () => void {
  return subscribeToSnapshots<CausalPage>(url, token, {
    maxBytes: options.maxBytes,
    timeoutMs: options.timeoutMs,
    parse: parseCausalFrame,
    sequence: causalSequence,
    onSnapshot: options.onSnapshot,
    onError: options.onError,
    unavailable: "Causal trace stream unavailable",
    timedOut: "Causal trace stream timed out",
    disconnected: "Causal trace stream disconnected",
    outOfSequence: "Trace sequence changed; fresh snapshot required",
  });
}
