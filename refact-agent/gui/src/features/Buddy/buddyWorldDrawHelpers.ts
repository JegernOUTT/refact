import type {
  BuddyWorldLayer,
  BuddyWorldState,
  BuddyWorldTone,
} from "./buddyWorldModel";
import type { Palette } from "./types";

export const TAU = Math.PI * 2;
export const BUDDY_WORLD_HOME_HOTSPOT = { x: 8.5, y: 67 } as const;

const UINT_MAX = 4_294_967_295;

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

export function finiteOrZero(value: number): number {
  return Number.isFinite(value) ? value : 0;
}

export function finiteOr(
  value: number | null | undefined,
  fallback: number,
): number {
  return typeof value === "number" && Number.isFinite(value) ? value : fallback;
}

export function safeDimension(value: number, fallback: number): number {
  return Math.max(1, finiteOr(value, fallback));
}

export function safeFrame(value: number): number {
  return finiteOr(value, 0);
}

export function clamp(value: number, min: number, max: number): number {
  const low = Math.min(min, max);
  const high = Math.max(min, max);
  return Math.max(low, Math.min(high, finiteOr(value, low)));
}

export function clamp01(value: number): number {
  return clamp(value, 0, 1);
}

export function clampAlpha(value: number): number {
  return clamp01(value);
}

export function pctX(width: number, value: number | null | undefined): number {
  return finiteOrZero((safeDimension(width, 720) * finiteOr(value, 0)) / 100);
}

export function pctY(height: number, value: number | null | undefined): number {
  return finiteOrZero((safeDimension(height, 260) * finiteOr(value, 0)) / 100);
}

export function seededUnit(seed: number, salt: number): number {
  let value = (finiteOrZero(seed) + Math.imul(salt + 1, 0x9e3779b9)) >>> 0;
  value ^= value >>> 16;
  value = Math.imul(value, 0x85ebca6b) >>> 0;
  value ^= value >>> 13;
  value = Math.imul(value, 0xc2b2ae35) >>> 0;
  value ^= value >>> 16;
  return (value >>> 0) / UINT_MAX;
}

export function seededRange(
  seed: number,
  salt: number,
  min: number,
  max: number,
): number {
  return lerp(min, max, seededUnit(seed, salt));
}

export function lerp(from: number, to: number, progress: number): number {
  return finiteOrZero(
    finiteOr(from, 0) +
      (finiteOr(to, 0) - finiteOr(from, 0)) * clamp01(progress),
  );
}

export function wave(
  frame: number,
  divisor: number,
  offset: number,
  amplitude: number,
  reducedMotion = false,
): number {
  if (reducedMotion) return 0;
  const safeDivisor = Math.max(1, Math.abs(finiteOr(divisor, 1)));
  return (
    Math.sin(safeFrame(frame) / safeDivisor + finiteOr(offset, 0)) *
    finiteOr(amplitude, 0)
  );
}

export function countForMotion(
  standard: number,
  compact: boolean,
  reducedMotion: boolean,
): number {
  const compactCount = compact ? Math.ceil(standard * 0.68) : standard;
  const reducedCount = reducedMotion
    ? Math.ceil(compactCount * 0.56)
    : compactCount;
  return Math.max(1, reducedCount);
}

export function alphaForMotion(alpha: number, reducedMotion: boolean): number {
  return clampAlpha(reducedMotion ? alpha * 0.62 : alpha);
}

export function toneColor(tone: BuddyWorldTone | undefined): string {
  switch (tone) {
    case "good":
      return "#22C55E";
    case "warning":
      return "#F59E0B";
    case "danger":
      return "#EF4444";
    case "neutral":
    default:
      return "#60A5FA";
  }
}

export function worldPhase(world: BuddyWorldState): BuddyWorldState["phase"] {
  switch (world.phase) {
    case "morning":
    case "day":
    case "evening":
    case "night":
      return world.phase;
    default:
      return "day";
  }
}

export function worldWeather(
  world: BuddyWorldState,
): BuddyWorldState["weather"] {
  switch (world.weather) {
    case "clear":
    case "aurora":
    case "busy":
    case "wind":
    case "rain":
    case "storm":
    case "dream":
      return world.weather;
    default:
      return "clear";
  }
}

export function worldPaletteHint(
  world: BuddyWorldState,
): BuddyWorldState["atmosphere"]["paletteHint"] {
  switch (world.atmosphere.paletteHint) {
    case "dawn":
    case "day":
    case "dusk":
    case "night":
    case "dream":
    case "storm":
      return world.atmosphere.paletteHint;
    default:
      return worldPhase(world) === "night" ? "night" : "day";
  }
}

export function worldIntensity(world: BuddyWorldState): number {
  return clamp01(world.atmosphere.intensity);
}

export function worldLayers(world: BuddyWorldState): BuddyWorldLayer[] {
  return world.atmosphere.layers;
}

export function hasWorldLayer(
  world: BuddyWorldState,
  layer: BuddyWorldLayer,
): boolean {
  return worldLayers(world).includes(layer);
}

export function worldObjects(
  world: BuddyWorldState,
): BuddyWorldState["objects"] {
  return Array.isArray(world.objects) ? world.objects : [];
}

export function objectAnchor(
  args: DrawBuddyWorldBaseArgs,
  id: string,
  fallback: Point,
): Point {
  const item = worldObjects(args.world).find((object) => object.id === id);
  return {
    x: pctX(args.width, item?.interactionX ?? item?.x ?? fallback.x),
    y: pctY(args.height, item?.interactionY ?? item?.y ?? fallback.y),
  };
}

export function fillRect(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  width: number,
  height: number,
  fillStyle: string | CanvasGradient | CanvasPattern,
  alpha = 1,
): void {
  ctx.save();
  ctx.globalAlpha = clampAlpha(alpha);
  ctx.fillStyle = fillStyle;
  ctx.fillRect(
    Math.round(finiteOrZero(x)),
    Math.round(finiteOrZero(y)),
    Math.max(1, Math.round(Math.abs(finiteOr(width, 1)))),
    Math.max(1, Math.round(Math.abs(finiteOr(height, 1)))),
  );
  ctx.restore();
}

export function fillPixelRect(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  width: number,
  height: number,
  color: string,
  alpha = 1,
): void {
  fillRect(ctx, x, y, width, height, color, alpha);
}

export function fillCircle(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  radius: number,
  color: string,
  alpha = 1,
): void {
  ctx.save();
  ctx.globalAlpha = clampAlpha(alpha);
  ctx.fillStyle = color;
  ctx.beginPath();
  ctx.arc(
    finiteOrZero(x),
    finiteOrZero(y),
    Math.max(0, finiteOr(radius, 0)),
    0,
    TAU,
  );
  ctx.fill();
  ctx.restore();
}

export function strokeCircle(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  radius: number,
  color: string,
  width: number,
  alpha = 1,
): void {
  ctx.save();
  ctx.globalAlpha = clampAlpha(alpha);
  ctx.strokeStyle = color;
  ctx.lineWidth = Math.max(0.5, finiteOr(width, 1));
  ctx.beginPath();
  ctx.arc(
    finiteOrZero(x),
    finiteOrZero(y),
    Math.max(0, finiteOr(radius, 0)),
    0,
    TAU,
  );
  ctx.stroke();
  ctx.restore();
}

export function fillEllipse(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  radiusX: number,
  radiusY: number,
  color: string,
  alpha = 1,
): void {
  ctx.save();
  ctx.globalAlpha = clampAlpha(alpha);
  ctx.fillStyle = color;
  ctx.beginPath();
  ctx.ellipse(
    finiteOrZero(x),
    finiteOrZero(y),
    Math.max(0, finiteOr(radiusX, 0)),
    Math.max(0, finiteOr(radiusY, 0)),
    0,
    0,
    TAU,
  );
  ctx.fill();
  ctx.restore();
}

export function strokeEllipse(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  radiusX: number,
  radiusY: number,
  color: string,
  width: number,
  alpha = 1,
): void {
  ctx.save();
  ctx.globalAlpha = clampAlpha(alpha);
  ctx.strokeStyle = color;
  ctx.lineWidth = Math.max(0.5, finiteOr(width, 1));
  ctx.beginPath();
  ctx.ellipse(
    finiteOrZero(x),
    finiteOrZero(y),
    Math.max(0, finiteOr(radiusX, 0)),
    Math.max(0, finiteOr(radiusY, 0)),
    0,
    0,
    TAU,
  );
  ctx.stroke();
  ctx.restore();
}

export function strokeLine(
  ctx: CanvasRenderingContext2D,
  from: Point,
  to: Point,
  color: string,
  width: number,
  alpha = 1,
): void {
  ctx.save();
  ctx.globalAlpha = clampAlpha(alpha);
  ctx.strokeStyle = color;
  ctx.lineWidth = Math.max(0.5, finiteOr(width, 1));
  ctx.lineCap = "round";
  ctx.beginPath();
  ctx.moveTo(finiteOrZero(from.x), finiteOrZero(from.y));
  ctx.lineTo(finiteOrZero(to.x), finiteOrZero(to.y));
  ctx.stroke();
  ctx.restore();
}

export function strokeBezier(
  ctx: CanvasRenderingContext2D,
  from: Point,
  cp1: Point,
  cp2: Point,
  to: Point,
  color: string,
  width: number,
  alpha = 1,
): void {
  ctx.save();
  ctx.globalAlpha = clampAlpha(alpha);
  ctx.strokeStyle = color;
  ctx.lineWidth = Math.max(0.5, finiteOr(width, 1));
  ctx.lineCap = "round";
  ctx.beginPath();
  ctx.moveTo(finiteOrZero(from.x), finiteOrZero(from.y));
  ctx.bezierCurveTo(
    finiteOrZero(cp1.x),
    finiteOrZero(cp1.y),
    finiteOrZero(cp2.x),
    finiteOrZero(cp2.y),
    finiteOrZero(to.x),
    finiteOrZero(to.y),
  );
  ctx.stroke();
  ctx.restore();
}

export function drawPixelText(
  ctx: CanvasRenderingContext2D,
  text: string,
  x: number,
  y: number,
  color: string,
  alpha = 1,
  align: CanvasTextAlign = "center",
): void {
  ctx.save();
  ctx.globalAlpha = clampAlpha(alpha);
  ctx.font = "10px monospace";
  ctx.textAlign = align;
  ctx.textBaseline = "middle";
  ctx.fillStyle = color;
  ctx.fillText(text, finiteOrZero(x), finiteOrZero(y));
  ctx.restore();
}

export function drawCloud(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  scale: number,
  color: string,
  alpha = 1,
): void {
  const safeScale = Math.max(0.1, finiteOr(scale, 1));
  fillPixelRect(
    ctx,
    x,
    y + 8 * safeScale,
    34 * safeScale,
    10 * safeScale,
    color,
    alpha,
  );
  fillPixelRect(
    ctx,
    x + 6 * safeScale,
    y + 2 * safeScale,
    10 * safeScale,
    8 * safeScale,
    color,
    alpha,
  );
  fillPixelRect(
    ctx,
    x + 16 * safeScale,
    y,
    12 * safeScale,
    10 * safeScale,
    color,
    alpha,
  );
  fillPixelRect(
    ctx,
    x + 28 * safeScale,
    y + 5 * safeScale,
    9 * safeScale,
    9 * safeScale,
    color,
    alpha,
  );
}

export function drawSpark(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  size: number,
  color: string,
  alpha = 1,
): void {
  const safeSize = Math.max(1, finiteOr(size, 2));
  fillCircle(ctx, x, y, safeSize * 2.2, color, alpha * 0.14);
  fillPixelRect(
    ctx,
    x - safeSize / 2,
    y - safeSize / 2,
    safeSize,
    safeSize,
    color,
    alpha,
  );
  fillPixelRect(
    ctx,
    x - safeSize * 1.35,
    y,
    safeSize * 2.7,
    1,
    color,
    alpha * 0.72,
  );
  fillPixelRect(
    ctx,
    x,
    y - safeSize * 1.35,
    1,
    safeSize * 2.7,
    color,
    alpha * 0.72,
  );
}
