import React from "react";
import type { BuddyControl, BuddyEvent, BuddyScenePose, BuddySemanticState, BubblePosition, Palette, Stage } from "./types";
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
    speechText?: string | null;
    speechControls?: BuddyControl[];
    speechIntent?: string;
    onCanvasEvent: (event: BuddyEvent) => void;
    onSpeechControl?: (control: BuddyControl) => void;
}
export declare const BuddyCharacter: React.FC<BuddyCharacterProps>;
export {};
