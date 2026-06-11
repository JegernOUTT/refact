import type { BubblePosition } from "./types";

const LONG_COMPACT_SPEECH_LENGTH = 72;
const SIDE_PREFERRED_SPEECH_LENGTH = 30;
const HIGH_SCENE_Y_THRESHOLD = 84;

export function bubblePositionForSceneX(
  x: number,
  compact = false,
  speechText: string | null = null,
  sceneY: number | null = null,
): BubblePosition {
  const length = speechText?.length ?? 0;
  if (compact && length > LONG_COMPACT_SPEECH_LENGTH) {
    return "top";
  }
  if (
    length > SIDE_PREFERRED_SPEECH_LENGTH ||
    (sceneY !== null && sceneY < HIGH_SCENE_Y_THRESHOLD)
  ) {
    return x <= 50 ? "right" : "left";
  }
  if (x < 42) return "right";
  if (x > 58) return "left";
  return "top";
}
