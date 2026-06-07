import type { BuddyControl, BuddyOpportunity, BuddyRuntimeEvent, BuddySpeechItem, BuddySuggestion } from "./types";
export type BuddySceneSpeechSource = "speech" | "runtime" | "suggestion" | "opportunity";
export interface BuddySceneSpeech {
    id: string;
    text: string;
    controls: BuddyControl[];
    mustShow?: boolean;
    chat_id?: string;
    speech_intent?: string;
    source: BuddySceneSpeechSource;
    runtimeEventId?: string;
    suggestionId?: string;
    opportunityId?: string;
}
export declare function formatBuddyRuntimeEventText(event: BuddyRuntimeEvent): string;
export declare function isBuddySpeechExpired(speech: BuddySpeechItem, nowMs?: number): boolean;
export declare function compareBuddyRuntimeEvents(left: BuddyRuntimeEvent, right: BuddyRuntimeEvent): number;
export declare function buildBuddySceneSpeech(args: {
    activeSpeech: BuddySpeechItem | null;
    nowPlaying: BuddyRuntimeEvent | null;
    runtimeQueue: BuddyRuntimeEvent[];
    activeSuggestion?: BuddySuggestion | null;
    activeOpportunities?: BuddyOpportunity[];
}): BuddySceneSpeech | null;
export declare function pickBuddySceneSpeechCandidate(candidates: BuddySceneSpeech[]): BuddySceneSpeech | null;
export declare function buildBuddySceneSpeechCandidates(args: {
    nowPlaying: BuddyRuntimeEvent | null;
    runtimeQueue: BuddyRuntimeEvent[];
    activeSuggestion?: BuddySuggestion | null;
    activeOpportunities?: BuddyOpportunity[];
}): BuddySceneSpeech[];
