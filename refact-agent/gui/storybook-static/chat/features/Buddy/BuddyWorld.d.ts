import React from "react";
import type { BuddyCareAction, BuddyControl, BuddyEvent, BuddyPage, BuddyPetState, BuddyPulse, BuddyQuest, BuddyRuntimeEvent, BuddySemanticState, Palette, Stage } from "./types";
interface BuddyWorldProps {
    palette: Palette;
    stage: Stage;
    state: BuddySemanticState;
    pulse: BuddyPulse | null | undefined;
    pet: BuddyPetState | undefined;
    nowPlaying: BuddyRuntimeEvent | null;
    activeQuest: BuddyQuest | null;
    activeSpeech: {
        text: string;
        controls: BuddyControl[];
        chat_id?: string;
        speech_intent?: string;
    } | null;
    setupNeeded: boolean;
    compact?: boolean;
    homeDoorDisabled?: boolean;
    onCanvasEvent: (event: BuddyEvent) => void;
    onCare: (action: BuddyCareAction, toy?: string) => void;
    onOpenPage: (page: BuddyPage) => void;
    onRunMode: (mode: string) => void;
    onDismissSetup: () => void;
    onSpeechControl: (control: BuddyControl) => void;
    now?: Date;
}
export declare const BuddyWorld: React.FC<BuddyWorldProps>;
export {};
