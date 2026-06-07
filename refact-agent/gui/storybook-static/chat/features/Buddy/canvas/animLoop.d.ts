import type { BuddyAnimState, BuddySemanticState, BuddyEvent } from "../types";
export declare function triggerSignalAnimation(anim: BuddyAnimState, signalType: string, emit: (e: BuddyEvent) => void): void;
export declare function updateSceneAnimation(anim: BuddyAnimState, scene: string, variant: string): void;
export declare function stepAnimFrame(anim: BuddyAnimState, semantic: BuddySemanticState, emit: (e: BuddyEvent) => void): void;
export declare function handlePet(anim: BuddyAnimState, canvasX: number, canvasY: number, emit: (e: BuddyEvent) => void, stage?: number): void;
