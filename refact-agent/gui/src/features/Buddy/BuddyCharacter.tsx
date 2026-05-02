import React, { type CSSProperties } from "react";
import { BuddyCanvas } from "./BuddyCanvas";
import type {
  BuddyControl,
  BuddyEvent,
  BuddyScenePose,
  BuddySemanticState,
  BubblePosition,
  Palette,
  Stage,
} from "./types";
import styles from "./BuddyWorld.module.css";

interface BuddyCharacterProps {
  state: BuddySemanticState;
  stage: Stage;
  palette: Palette;
  displaySize: number;
  showStageBadge?: boolean;
  bubblePosition?: BubblePosition;
  randomizeBubblePosition?: boolean;
  sceneXPercent?: number;
  sceneYPercent?: number;
  sceneDepthScale?: number;
  scenePose?: BuddyScenePose;
  speechText?: string | null;
  speechControls?: BuddyControl[];
  onCanvasEvent: (event: BuddyEvent) => void;
  onSpeechControl?: (control: BuddyControl) => void;
}

type BuddyCharacterStyle = CSSProperties & {
  "--buddy-scene-scale"?: number;
};

function buildBuddyCharacterStyle(args: {
  sceneXPercent: number | undefined;
  sceneYPercent: number | undefined;
  sceneDepthScale: number | undefined;
}): BuddyCharacterStyle | undefined {
  const style: BuddyCharacterStyle = {};
  if (typeof args.sceneXPercent === "number") {
    style.left = `${args.sceneXPercent}%`;
  }
  if (typeof args.sceneYPercent === "number") {
    style.bottom = `${100 - args.sceneYPercent}%`;
  }
  if (typeof args.sceneDepthScale === "number") {
    style["--buddy-scene-scale"] = args.sceneDepthScale;
  }
  return Object.keys(style).length > 0 ? style : undefined;
}

export const BuddyCharacter: React.FC<BuddyCharacterProps> = ({
  state,
  stage,
  palette,
  displaySize,
  showStageBadge = false,
  bubblePosition = "top",
  randomizeBubblePosition = false,
  sceneXPercent,
  sceneYPercent,
  sceneDepthScale,
  scenePose = "idle",
  speechText,
  speechControls,
  onCanvasEvent,
  onSpeechControl,
}) => (
  <div
    className={styles.characterAnchor}
    style={buildBuddyCharacterStyle({
      sceneXPercent,
      sceneYPercent,
      sceneDepthScale,
    })}
    data-bubble-position={bubblePosition}
    data-pose={scenePose}
    data-randomize-bubble-position={String(randomizeBubblePosition)}
    data-testid="buddy-world-character"
  >
    <div className={styles.characterBody} data-pose={scenePose}>
      <BuddyCanvas
        state={state}
        onEvent={onCanvasEvent}
        displaySize={displaySize}
        speechOverride={speechText}
        speechControls={speechControls}
        onSpeechControlClick={onSpeechControl}
        bubblePosition={bubblePosition}
        randomizeBubblePosition={randomizeBubblePosition}
      />
      {showStageBadge && (
        <div
          className={styles.stageBadge}
          style={{ borderColor: palette.body, color: palette.body }}
        >
          {stage.emoji} {stage.name}
        </div>
      )}
    </div>
  </div>
);
