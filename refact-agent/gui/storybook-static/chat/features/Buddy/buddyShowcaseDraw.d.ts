import type { BuddyShowcaseRun, Palette } from "./types";
import type { BuddyWorldState } from "./buddyWorldModel";
export interface DrawShowcaseEventArgs {
    ctx: CanvasRenderingContext2D;
    run: BuddyShowcaseRun;
    world: BuddyWorldState;
    palette: Palette;
    frame: number;
    width: number;
    height: number;
    compact: boolean;
    reducedMotion: boolean;
    nowMs?: number;
}
export declare function drawShowcaseEvent(args: DrawShowcaseEventArgs): void;
