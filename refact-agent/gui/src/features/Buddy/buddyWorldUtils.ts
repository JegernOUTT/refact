import type { BubblePosition } from "./types";

const LONG_COMPACT_SPEECH_LENGTH = 72;

export function bubblePositionForSceneX(
  x: number,
  compact = false,
  speechText: string | null = null,
): BubblePosition {
  if (compact && (speechText?.length ?? 0) > LONG_COMPACT_SPEECH_LENGTH) {
    return "top";
  }
  if (x < 42) return "right";
  if (x > 58) return "left";
  return "top";
}
