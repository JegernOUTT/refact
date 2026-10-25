import { workSequence, type WorkPage } from "./workTypes";
import { parseWorkFrame } from "./workSse";
import { subscribeToSnapshots } from "./snapshotStream";

export type WorkStreamOptions = {
  timeoutMs: number;
  onSnapshot: (page: WorkPage) => void;
  onError: (reason: string) => void;
};

export function subscribeToWork(
  url: string,
  token: string | null,
  options: WorkStreamOptions,
): () => void {
  return subscribeToSnapshots<WorkPage>(url, token, {
    timeoutMs: options.timeoutMs,
    parse: parseWorkFrame,
    sequence: workSequence,
    onSnapshot: options.onSnapshot,
    onError: options.onError,
    unavailable: "Work stream unavailable",
    timedOut: "Work stream timed out",
    disconnected: "Work stream disconnected",
    outOfSequence: "Work sequence changed; fresh snapshot required",
  });
}
