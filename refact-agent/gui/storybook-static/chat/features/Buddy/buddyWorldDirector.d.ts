import type { BuddyScenePose } from "./types";
import type { BuddyWorldState } from "./buddyWorldModel";
export type BuddyWorldIntentKind = "morning_stretch" | "evening_tidy" | "night_watch" | "rest_home" | "inspect_memory" | "shelve_memory" | "inspect_provider" | "stabilize_crystal" | "channel_runtime" | "watch_observatory" | "seek_food" | "seek_toy" | "receive_affection" | "wander_curiously" | "celebrate_recovery";
export interface BuddyWorldIntent {
    id: string;
    kind: BuddyWorldIntentKind;
    targetX: number;
    targetY: number;
    depthScale: number;
    pose: BuddyScenePose;
    speech: string | null;
    speechKind: "charm" | "actionable";
    durationMs: number;
    priority: number;
    objectId?: string;
}
export interface ChooseBuddyWorldIntentArgs {
    world: BuddyWorldState;
    previousIntent: BuddyWorldIntent | null;
    nowMs: number;
    activeSpeechVisible: boolean;
    showcaseActive: boolean;
    localReactionVisible: boolean;
    reducedMotion: boolean;
    recentIntentKinds?: readonly BuddyWorldIntentKind[];
}
export declare function chooseBuddyWorldIntent(args: ChooseBuddyWorldIntentArgs): BuddyWorldIntent | null;
