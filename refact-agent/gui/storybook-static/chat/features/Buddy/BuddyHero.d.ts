import React from "react";
import type { BuddyCareAction, BuddyControl, BuddyEvent, BuddyRuntimeEvent, BuddySemanticState, BuddySpeechItem, Palette, Stage } from "./types";
interface BuddyHeroProps {
    palette: Palette;
    stage: Stage;
    statusText: string;
    state: BuddySemanticState;
    onCanvasEvent: (event: BuddyEvent) => void;
    activeSpeech: BuddySpeechItem | null;
    nowPlaying: BuddyRuntimeEvent | null;
    setupNeeded: boolean;
    onRunMode: (mode: string) => void;
    onDismissSetup: () => void;
    onCare: (action: BuddyCareAction, toy?: string) => void;
    onSpeechControl: (control: BuddyControl) => void;
}
export declare const BuddyHero: React.FC<BuddyHeroProps>;
export {};
