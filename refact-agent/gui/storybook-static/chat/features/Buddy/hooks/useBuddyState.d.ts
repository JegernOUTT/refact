import type { BuddySemanticState, BuddyEvent } from "../types";
export interface BuddyStateHandle {
    state: BuddySemanticState;
    signal: (signalType: string) => void;
    addXP: (amount: number) => void;
    pet: () => void;
    rename: (name: string) => void;
    nextPalette: () => void;
    reset: () => void;
    handleCanvasEvent: (event: BuddyEvent) => void;
    onBuddyEvent?: (event: BuddyEvent) => void;
}
export declare function useBuddyState(initialState?: BuddySemanticState, onBuddyEvent?: (event: BuddyEvent) => void): BuddyStateHandle;
