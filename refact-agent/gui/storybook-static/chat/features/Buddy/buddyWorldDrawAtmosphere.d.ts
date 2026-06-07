import type { BuddyWorldState } from "./buddyWorldModel";
import { type DrawBuddyWorldBaseArgs } from "./buddyWorldDrawHelpers";
export declare function drawSkyGradient(args: DrawBuddyWorldBaseArgs): void;
export declare function shouldDrawStarField(world: BuddyWorldState): boolean;
export declare function drawStarField(args: DrawBuddyWorldBaseArgs): void;
export declare function drawObservatoryStructures(args: DrawBuddyWorldBaseArgs): void;
export declare function drawCelestial(args: DrawBuddyWorldBaseArgs): void;
export declare function drawAmbientLayers(args: DrawBuddyWorldBaseArgs): void;
export declare function drawWeatherAtmosphere(args: DrawBuddyWorldBaseArgs): void;
