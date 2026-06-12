import type { BubblePosition } from "./types";

const SIDE_PREFERRED_SPEECH_LENGTH = 30;
const COMPACT_SIDE_PREFERRED_SPEECH_LENGTH = 24;
const HIGH_SCENE_Y_THRESHOLD = 84;

export function bubblePositionForSceneX(
  x: number,
  compact = false,
  speechText: string | null = null,
  sceneY: number | null = null,
): BubblePosition {
  const length = speechText?.length ?? 0;
  const sidePreferredLength = compact
    ? COMPACT_SIDE_PREFERRED_SPEECH_LENGTH
    : SIDE_PREFERRED_SPEECH_LENGTH;
  if (
    length > sidePreferredLength ||
    (sceneY !== null && sceneY < HIGH_SCENE_Y_THRESHOLD)
  ) {
    return x <= 50 ? "right" : "left";
  }
  if (x < 42) return "right";
  if (x > 58) return "left";
  return "top";
}
