import React from "react";
import type { BuddyControl, BuddyNeeds, BuddyPersonalityProfile, BuddyQuest, BuddySettings } from "./types";
export interface NeedRow {
    key: keyof BuddyNeeds;
    label: string;
    value: number;
    fill: number;
    invert?: boolean;
}
interface BuddyPersonalityPanelProps {
    personality: BuddyPersonalityProfile | undefined;
    needRows: NeedRow[];
    unlockedSkills: string[];
    activeQuest: BuddyQuest | null;
    name: string;
    settings: BuddySettings | undefined;
    isSavingSettings: boolean;
    onQuestControl: (control: BuddyControl) => void;
    onReroll: () => void;
    onToggleProactive: () => void;
    onPromptChange: (prompt: string | null) => void;
}
export declare const BuddyPersonalityPanel: React.FC<BuddyPersonalityPanelProps>;
export {};
