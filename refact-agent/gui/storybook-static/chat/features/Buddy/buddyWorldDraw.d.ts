import type { BuddyWorldState } from "./buddyWorldModel";
import type { Palette } from "./types";
export interface DrawBuddyWorldArgs {
    ctx: CanvasRenderingContext2D;
    world: BuddyWorldState;
    palette: Palette;
    frame: number;
    width: number;
    height: number;
    compact: boolean;
    reducedMotion: boolean;
}
export declare function drawBuddyWorld(args: DrawBuddyWorldArgs): void;
