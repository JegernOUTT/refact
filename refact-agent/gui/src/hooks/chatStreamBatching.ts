import type { DeltaOp } from "../services/refact/chatSubscription";

export const MAX_MERGED_DELTA_OPS = 256;
export const MAX_BUFFERED_STREAM_TEXT_UNITS = 2_000_000;

const ACTIVE_SMALL_STREAM_TEXT_UNITS = 8_192;
const ACTIVE_SMALL_STREAM_FLUSH_MS = 50;
const ACTIVE_LARGE_STREAM_FLUSH_MS = 125;
const BACKGROUND_STREAM_FLUSH_MS = 750;
const ACTIVE_SUBCHAT_FLUSH_MS = 125;
const BACKGROUND_SUBCHAT_FLUSH_MS = 750;

export function streamDeltaTextUnits(ops: DeltaOp[]): number {
  let textUnits = 0;
  for (const op of ops) {
    if (
      op.op === "append_content" ||
      op.op === "append_reasoning" ||
      op.op === "set_reasoning"
    ) {
      textUnits += op.text.length;
    }
  }
  return textUnits;
}

export function streamDeltaFlushDelayMs(
  isActive: boolean,
  streamedTextUnits: number,
): number {
  if (!isActive) return BACKGROUND_STREAM_FLUSH_MS;
  return streamedTextUnits < ACTIVE_SMALL_STREAM_TEXT_UNITS
    ? ACTIVE_SMALL_STREAM_FLUSH_MS
    : ACTIVE_LARGE_STREAM_FLUSH_MS;
}

export function subchatFlushDelayMs(isActive: boolean): number {
  return isActive ? ACTIVE_SUBCHAT_FLUSH_MS : BACKGROUND_SUBCHAT_FLUSH_MS;
}
