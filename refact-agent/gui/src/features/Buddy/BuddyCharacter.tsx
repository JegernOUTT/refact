import React, { type CSSProperties } from "react";
import { BuddyCanvas } from "./BuddyCanvas";
import type {
  BuddyControl,
  BuddyCursorBridge,
  BuddyEnvContext,
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
  compactBubble?: boolean;
  sceneXPercent?: number;
  sceneYPercent?: number;
  sceneDepthScale?: number;
  scenePose?: BuddyScenePose;
  traveling?: boolean;
  arrived?: boolean;
  travelDirection?: "left" | "right";
  envContext?: BuddyEnvContext | null;
  spritePointer?: boolean;
  cursorBridgeRef?: React.MutableRefObject<BuddyCursorBridge | null>;
  speechText?: string | null;
  speechControls?: BuddyControl[];
  speechIntent?: string;
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
  displaySize: number;
}): BuddyCharacterStyle | undefined {
  const style: BuddyCharacterStyle = {};
  if (typeof args.sceneXPercent === "number") {
    style.left = `${args.sceneXPercent}%`;
  }
  if (typeof args.sceneYPercent === "number") {
    const feetOffsetPx = Math.round(args.displaySize * 0.3);
    style.bottom = `calc(${100 - args.sceneYPercent}% - ${feetOffsetPx}px)`;
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
  compactBubble = false,
  sceneXPercent,
  sceneYPercent,
  sceneDepthScale,
  scenePose = "idle",
  traveling = false,
  arrived = false,
  travelDirection = "right",
  envContext,
  spritePointer = false,
  cursorBridgeRef,
  speechText,
  speechControls,
  speechIntent,
  onCanvasEvent,
  onSpeechControl,
}) => (
  <div
    className={styles.characterAnchor}
    style={buildBuddyCharacterStyle({
      sceneXPercent,
      sceneYPercent,
      sceneDepthScale,
      displaySize,
    })}
    data-bubble-position={bubblePosition}
    data-compact-bubble={String(compactBubble)}
    data-pose={scenePose}
    data-traveling={String(traveling)}
    data-travel-direction={travelDirection}
    data-randomize-bubble-position={String(randomizeBubblePosition)}
    data-testid="buddy-world-character"
  >
    <div
      className={styles.characterBody}
      data-pose={scenePose}
      data-traveling={String(traveling)}
      data-arrived={String(arrived)}
      data-travel-direction={travelDirection}
    >
      <BuddyCanvas
        state={state}
        onEvent={onCanvasEvent}
        displaySize={displaySize}
        envContext={envContext}
        spritePointer={spritePointer}
        cursorBridgeRef={cursorBridgeRef}
        speechOverride={speechText}
        speechControls={speechControls}
        speechIntent={speechIntent}
        onSpeechControlClick={onSpeechControl}
        bubblePosition={bubblePosition}
        randomizeBubblePosition={randomizeBubblePosition}
        compactBubble={compactBubble}
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
