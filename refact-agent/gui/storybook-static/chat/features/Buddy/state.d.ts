import type { BuddySemanticState, BuddyAnimState } from "./types";
export declare function randomName(): string;
export declare function randomPaletteIndex(): number;
export declare function createInitialSemanticState(): BuddySemanticState;
export declare function createInitialAnimState(): BuddyAnimState;
export type SemanticAction = {
    kind: "signal";
    signalType: string;
} | {
    kind: "add_xp";
    amount: number;
} | {
    kind: "pet";
} | {
    kind: "rename";
    name: string;
} | {
    kind: "next_palette";
} | {
    kind: "reset";
} | {
    kind: "patch";
    patch: Partial<BuddySemanticState>;
};
export declare function reduceSemanticState(state: BuddySemanticState, action: SemanticAction): BuddySemanticState;
