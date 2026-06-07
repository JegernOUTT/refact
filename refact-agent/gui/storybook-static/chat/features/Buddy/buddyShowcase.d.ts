import type { BuddyPetState, BuddyPulse, BuddyRuntimeEvent, BuddyScenePose, BuddyShowcaseKind, BuddyShowcasePhase, BuddyShowcaseRun, BuddyShowcaseTarget } from "./types";
import type { BuddyWorldPhase, BuddyWorldWeather } from "./buddyWorldModel";
export declare const BUDDY_SHOWCASE_PHASE_DURATIONS_MS: Record<BuddyShowcasePhase, number>;
export declare const BUDDY_SHOWCASE_INITIAL_GRACE_MS = 30000;
export declare const BUDDY_SHOWCASE_IDLE_COOLDOWN_MS = 78000;
export declare const BUDDY_SHOWCASE_TRIGGER_COOLDOWN_MS = 18000;
export interface BuddyShowcaseDefinition {
    kind: BuddyShowcaseKind;
    targetId: string;
    targetSprite?: string;
    pose: BuddyScenePose;
    speech: (name: string) => string;
}
export declare const BUDDY_SHOWCASE_DEFINITIONS: Record<BuddyShowcaseKind, BuddyShowcaseDefinition>;
export type BuddyShowcaseChoice = BuddyShowcaseDefinition;
export interface BuddyShowcaseTargetCandidate extends BuddyShowcaseTarget {
    sprite?: string;
}
export interface BuddyShowcaseWorldContext {
    phase: BuddyWorldPhase;
    weather: BuddyWorldWeather;
}
export interface ChooseBuddyShowcaseArgs {
    targets: BuddyShowcaseTargetCandidate[];
    nowPlaying: BuddyRuntimeEvent | null;
    activeSpeechVisible: boolean;
    pet: BuddyPetState | undefined;
    nowMs: number;
    idleCooldownUntilMs?: number;
    runtimeCooldownUntilMs?: number;
    idleGraceUntilMs?: number;
    lastShowcaseKind?: BuddyShowcaseKind | null;
    runtimeShowcaseEventIds?: readonly string[];
    strongRuntimeTrigger?: boolean;
    identityName?: string;
    world?: BuddyShowcaseWorldContext;
    pulse?: BuddyPulse | null;
}
export interface CreateBuddyShowcaseRunArgs extends ChooseBuddyShowcaseArgs {
    idPrefix?: string;
}
export interface AdvanceBuddyShowcasePhaseArgs {
    run: BuddyShowcaseRun;
    nowMs: number;
}
export declare function hasBuddyShowcaseRuntimeTrigger(event: BuddyRuntimeEvent | null, nowMs?: number): boolean;
export declare function chooseBuddyShowcase(args: ChooseBuddyShowcaseArgs): BuddyShowcaseChoice | null;
export declare function createBuddyShowcaseSeed(args: {
    kind: BuddyShowcaseKind;
    nowMs: number;
    target: BuddyShowcaseTarget;
}): number;
export declare function createBuddyShowcaseRun(args: CreateBuddyShowcaseRunArgs): BuddyShowcaseRun | null;
export declare function advanceBuddyShowcasePhase(args: AdvanceBuddyShowcasePhaseArgs): BuddyShowcaseRun | null;
