import type { BuddyWorldLayer, BuddyWorldState, BuddyWorldTone } from "./buddyWorldModel";
import type { Palette } from "./types";
export declare const TAU: number;
export declare const BUDDY_WORLD_HOME_HOTSPOT: {
    readonly x: 8.5;
    readonly y: 67;
};
export interface Point {
    x: number;
    y: number;
}
export interface DrawBuddyWorldBaseArgs {
    ctx: CanvasRenderingContext2D;
    world: BuddyWorldState;
    palette: Palette;
    frame: number;
    width: number;
    height: number;
    compact: boolean;
    reducedMotion: boolean;
}
export declare function finiteOrZero(value: number): number;
export declare function finiteOr(value: number | null | undefined, fallback: number): number;
export declare function safeDimension(value: number, fallback: number): number;
export declare function safeFrame(value: number): number;
export declare function clamp(value: number, min: number, max: number): number;
export declare function clamp01(value: number): number;
export declare function clampAlpha(value: number): number;
export declare function pctX(width: number, value: number | null | undefined): number;
export declare function pctY(height: number, value: number | null | undefined): number;
export declare function seededUnit(seed: number, salt: number): number;
export declare function seededRange(seed: number, salt: number, min: number, max: number): number;
export declare function lerp(from: number, to: number, progress: number): number;
export declare function wave(frame: number, divisor: number, offset: number, amplitude: number, reducedMotion?: boolean): number;
export declare function countForMotion(standard: number, compact: boolean, reducedMotion: boolean): number;
export declare function alphaForMotion(alpha: number, reducedMotion: boolean): number;
export declare function toneColor(tone: BuddyWorldTone | undefined): string;
export declare function worldPhase(world: BuddyWorldState): BuddyWorldState["phase"];
export declare function worldWeather(world: BuddyWorldState): BuddyWorldState["weather"];
export declare function worldPaletteHint(world: BuddyWorldState): BuddyWorldState["atmosphere"]["paletteHint"];
export declare function worldIntensity(world: BuddyWorldState): number;
export declare function worldLayers(world: BuddyWorldState): BuddyWorldLayer[];
export declare function hasWorldLayer(world: BuddyWorldState, layer: BuddyWorldLayer): boolean;
export declare function worldObjects(world: BuddyWorldState): BuddyWorldState["objects"];
export declare function objectAnchor(args: DrawBuddyWorldBaseArgs, id: string, fallback: Point): Point;
export declare function fillRect(ctx: CanvasRenderingContext2D, x: number, y: number, width: number, height: number, fillStyle: string | CanvasGradient | CanvasPattern, alpha?: number): void;
export declare function fillPixelRect(ctx: CanvasRenderingContext2D, x: number, y: number, width: number, height: number, color: string, alpha?: number): void;
export declare function fillCircle(ctx: CanvasRenderingContext2D, x: number, y: number, radius: number, color: string, alpha?: number): void;
export declare function strokeCircle(ctx: CanvasRenderingContext2D, x: number, y: number, radius: number, color: string, width: number, alpha?: number): void;
export declare function fillEllipse(ctx: CanvasRenderingContext2D, x: number, y: number, radiusX: number, radiusY: number, color: string, alpha?: number): void;
export declare function strokeEllipse(ctx: CanvasRenderingContext2D, x: number, y: number, radiusX: number, radiusY: number, color: string, width: number, alpha?: number): void;
export declare function strokeLine(ctx: CanvasRenderingContext2D, from: Point, to: Point, color: string, width: number, alpha?: number): void;
export declare function strokeBezier(ctx: CanvasRenderingContext2D, from: Point, cp1: Point, cp2: Point, to: Point, color: string, width: number, alpha?: number): void;
export declare function drawPixelText(ctx: CanvasRenderingContext2D, text: string, x: number, y: number, color: string, alpha?: number, align?: CanvasTextAlign): void;
export declare function drawCloud(ctx: CanvasRenderingContext2D, x: number, y: number, scale: number, color: string, alpha?: number): void;
export declare function drawSpark(ctx: CanvasRenderingContext2D, x: number, y: number, size: number, color: string, alpha?: number): void;
