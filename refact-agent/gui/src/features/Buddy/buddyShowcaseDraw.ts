import type { BuddyShowcaseKind, BuddyShowcaseRun, Palette } from "./types";
import type { BuddyWorldState } from "./buddyWorldModel";
import { BUDDY_SHOWCASE_PHASE_DURATIONS_MS } from "./buddyShowcase";

const TAU = Math.PI * 2;
const UINT_MAX = 4_294_967_295;

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

type ShowcaseDrawer = (args: DrawShowcaseEventArgs) => void;

interface Point {
  x: number;
  y: number;
}

function pctX(width: number, value: number): number {
  return (width * value) / 100;
}

function pctY(height: number, value: number): number {
  return (height * value) / 100;
}

function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value));
}

function clamp01(value: number): number {
  return clamp(value, 0, 1);
}

function clampAlpha(value: number): number {
  return clamp01(value);
}

function lerp(from: number, to: number, progress: number): number {
  return from + (to - from) * progress;
}

function easeOut(progress: number): number {
  return 1 - (1 - progress) * (1 - progress);
}

function easeInOut(progress: number): number {
  return progress < 0.5
    ? 2 * progress * progress
    : 1 - Math.pow(-2 * progress + 2, 2) / 2;
}

function seededUnit(seed: number, salt: number): number {
  let value = (seed + Math.imul(salt + 1, 0x9e3779b9)) >>> 0;
  value ^= value >>> 16;
  value = Math.imul(value, 0x85ebca6b) >>> 0;
  value ^= value >>> 13;
  value = Math.imul(value, 0xc2b2ae35) >>> 0;
  value ^= value >>> 16;
  return (value >>> 0) / UINT_MAX;
}

function phaseProgress(run: BuddyShowcaseRun, nowMs?: number): number {
  const duration = BUDDY_SHOWCASE_PHASE_DURATIONS_MS[run.phase];
  const elapsed = (nowMs ?? Date.now()) - run.phaseStartedAtMs;
  return clamp01(elapsed / duration);
}

function eventAlpha(run: BuddyShowcaseRun, progress: number): number {
  switch (run.phase) {
    case "travel":
      return 0.14 + progress * 0.18;
    case "anticipate":
      return 0.34 + progress * 0.36;
    case "showcase":
      return 1;
    case "react":
      return 0.92 - progress * 0.18;
    case "cooldown":
      return 0.7 * (1 - progress);
  }
}

function reducedAlpha(alpha: number, reducedMotion: boolean): number {
  return reducedMotion ? alpha * 0.64 : alpha;
}

function timelineProgress(run: BuddyShowcaseRun, progress: number): number {
  switch (run.phase) {
    case "travel":
      return progress * 0.08;
    case "anticipate":
      return 0.08 + progress * 0.12;
    case "showcase":
      return progress;
    case "react":
      return 0.95 + progress * 0.05;
    case "cooldown":
      return 1;
  }
}

function fillPixelRect(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  width: number,
  height: number,
  color: string,
  alpha = 1,
): void {
  ctx.save();
  ctx.globalAlpha = clampAlpha(alpha);
  ctx.fillStyle = color;
  ctx.fillRect(
    Math.round(x),
    Math.round(y),
    Math.max(1, Math.round(width)),
    Math.max(1, Math.round(height)),
  );
  ctx.restore();
}

function fillCircle(
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
  ctx.arc(x, y, radius, 0, TAU);
  ctx.fill();
  ctx.restore();
}

function strokeLine(
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
  ctx.lineWidth = width;
  ctx.lineCap = "round";
  ctx.beginPath();
  ctx.moveTo(from.x, from.y);
  ctx.lineTo(to.x, to.y);
  ctx.stroke();
  ctx.restore();
}

function drawDottedLine(
  ctx: CanvasRenderingContext2D,
  from: Point,
  to: Point,
  color: string,
  alpha: number,
  dots: number,
): void {
  const safeDots = Math.max(2, dots);
  for (let index = 0; index < safeDots; index += 1) {
    const progress = index / (safeDots - 1);
    fillCircle(
      ctx,
      lerp(from.x, to.x, progress),
      lerp(from.y, to.y, progress),
      1.15,
      color,
      alpha * (0.45 + Math.sin(progress * Math.PI) * 0.3),
    );
  }
}

function drawSpark(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  size: number,
  color: string,
  alpha: number,
): void {
  fillCircle(ctx, x, y, size * 2.2, color, alpha * 0.14);
  fillPixelRect(ctx, x - size / 2, y - size / 2, size, size, color, alpha);
  fillPixelRect(ctx, x - size * 1.35, y, size * 2.7, 1, color, alpha * 0.72);
  fillPixelRect(ctx, x, y - size * 1.35, 1, size * 2.7, color, alpha * 0.72);
}

function memoryAnchor(args: DrawShowcaseEventArgs): Point {
  const x = pctX(args.width, args.run.target.x);
  const objectY = pctY(args.height, args.run.target.y);
  const buddyY = args.height * (args.compact ? 0.62 : 0.66);
  return { x, y: Math.max(objectY + 22, buddyY) };
}

function drawMemoryFireflyNight(args: DrawShowcaseEventArgs): void {
  const { ctx, run, frame, width, height, compact, reducedMotion, world } =
    args;
  const progress = phaseProgress(run, args.nowMs);
  const alpha = reducedAlpha(eventAlpha(run, progress), reducedMotion);
  if (alpha <= 0) return;

  const timeline = timelineProgress(run, progress);
  const origin = {
    x: pctX(width, run.target.x),
    y: pctY(height, run.target.y),
  };
  const anchor = memoryAnchor(args);
  const nightBoost = reducedMotion ? 0.94 : world.phase === "night" ? 1.16 : 1;
  const count = reducedMotion ? (compact ? 10 : 16) : compact ? 18 : 32;
  const speed = reducedMotion ? 180 : 72;

  fillCircle(
    ctx,
    origin.x,
    origin.y + 11,
    compact ? 32 : 46,
    "#FDE68A",
    alpha * 0.08 * nightBoost,
  );
  fillCircle(
    ctx,
    anchor.x,
    anchor.y - 8,
    compact ? 38 : 56,
    "#FBBF24",
    alpha * (0.09 + Math.sin(frame / 42) * 0.02) * nightBoost,
  );

  for (let index = 0; index < count; index += 1) {
    const baseAngle = seededUnit(run.seed, index * 11) * TAU;
    const startRadius = lerp(
      12,
      compact ? 42 : 62,
      seededUnit(run.seed, index * 11 + 1),
    );
    const orbitRadius = lerp(
      14,
      compact ? 38 : 52,
      seededUnit(run.seed, index * 11 + 2),
    );
    const lift = lerp(
      4,
      compact ? 22 : 32,
      seededUnit(run.seed, index * 11 + 3),
    );
    const start = {
      x: origin.x + Math.cos(baseAngle) * startRadius,
      y: origin.y + Math.sin(baseAngle) * startRadius * 0.65,
    };
    const hover = {
      x: anchor.x + Math.cos(baseAngle + 0.9) * orbitRadius * 0.48,
      y: anchor.y - lift + Math.sin(baseAngle) * orbitRadius * 0.2,
    };
    let x = start.x;
    let y = start.y;
    let particleAlpha =
      alpha * lerp(0.46, 0.94, seededUnit(run.seed, index * 11 + 4));
    const size = lerp(
      2,
      compact ? 3.2 : 4.2,
      seededUnit(run.seed, index * 11 + 5),
    );

    if (timeline < 0.34) {
      const local = easeInOut(clamp01(timeline / 0.34));
      x = lerp(start.x, hover.x, local);
      y = lerp(start.y, hover.y, local) - Math.sin(local * Math.PI) * 18;
      const trail = {
        x: lerp(start.x, x, 0.72),
        y: lerp(start.y, y, 0.72),
      };
      fillCircle(
        ctx,
        trail.x,
        trail.y,
        size * 1.4,
        "#FCD34D",
        particleAlpha * 0.16,
      );
    } else if (timeline < 0.72) {
      const local = clamp01((timeline - 0.34) / 0.38);
      const angle =
        baseAngle +
        local * TAU * (reducedMotion ? 0.45 : 1.35) +
        (reducedMotion ? 0 : frame / speed);
      const breathe = Math.sin(frame / (reducedMotion ? 110 : 34) + index) * 3;
      x = anchor.x + Math.cos(angle) * (orbitRadius + breathe);
      y = anchor.y - lift * 0.48 + Math.sin(angle) * (orbitRadius * 0.46);
    } else {
      const local = easeOut(clamp01((timeline - 0.72) / 0.28));
      const angle =
        baseAngle +
        TAU * (reducedMotion ? 0.4 : 1.4) +
        local * TAU * (reducedMotion ? 0.28 : 0.9);
      const radius = orbitRadius * (1 - local * 0.62);
      x =
        anchor.x +
        Math.cos(angle) * radius +
        Math.sin(local * TAU + baseAngle) * (compact ? 5 : 9);
      y =
        anchor.y -
        lift * 0.4 +
        Math.sin(angle) * radius * 0.32 -
        local * height * (compact ? 0.23 : 0.32);
      particleAlpha *= 1 - local * 0.76;
    }

    const color =
      index % 3 === 0 ? "#FEF3C7" : index % 3 === 1 ? "#FDE68A" : "#FBBF24";
    drawSpark(ctx, x, y, size, color, particleAlpha);
  }

  if (timeline > 0.55) {
    const swirlAlpha = alpha * clamp01((timeline - 0.55) / 0.45) * 0.28;
    const swirlCount = reducedMotion ? 1 : 3;
    for (let index = 0; index < swirlCount; index += 1) {
      const radius = compact ? 24 + index * 9 : 34 + index * 13;
      const y = anchor.y - 18 - index * 17 - timeline * 18;
      const from = {
        x: anchor.x - radius * 0.7,
        y: y + Math.sin(frame / 38 + index) * 3,
      };
      const to = {
        x: anchor.x + radius * 0.7,
        y: y - Math.cos(frame / 42 + index) * 3,
      };
      drawDottedLine(ctx, from, to, "#FDE68A", swirlAlpha, compact ? 8 : 13);
    }
  }
}

function constellationStars(
  args: DrawShowcaseEventArgs,
  center: Point,
  count: number,
): Point[] {
  const { run, width, height, compact } = args;
  const xRadius = width * (compact ? 0.17 : 0.2);
  const yRadius = height * (compact ? 0.1 : 0.12);
  return Array.from({ length: count }, (_, index) => {
    const angle =
      (index / count) * TAU + seededUnit(run.seed, 100 + index) * 0.72;
    const radius = lerp(0.42, 1, seededUnit(run.seed, 140 + index));
    return {
      x: clamp(center.x + Math.cos(angle) * xRadius * radius, 18, width - 18),
      y: clamp(
        center.y + Math.sin(angle) * yRadius * radius,
        18,
        height * 0.48,
      ),
    };
  });
}

function drawTelescope(
  ctx: CanvasRenderingContext2D,
  base: Point,
  palette: Palette,
  alpha: number,
): void {
  fillPixelRect(ctx, base.x - 26, base.y + 12, 52, 10, "#0F172A", alpha * 0.42);
  fillPixelRect(ctx, base.x - 12, base.y + 3, 24, 14, "#334155", alpha);
  fillPixelRect(
    ctx,
    base.x - 5,
    base.y - 14,
    10,
    20,
    palette.body,
    alpha * 0.86,
  );
  fillPixelRect(ctx, base.x + 2, base.y - 20, 34, 7, "#E0E7FF", alpha);
  fillPixelRect(ctx, base.x + 34, base.y - 22, 6, 11, "#FDE68A", alpha);
  strokeLine(
    ctx,
    { x: base.x - 13, y: base.y + 18 },
    { x: base.x - 25, y: base.y + 32 },
    "#94A3B8",
    2,
    alpha,
  );
  strokeLine(
    ctx,
    { x: base.x + 13, y: base.y + 18 },
    { x: base.x + 25, y: base.y + 32 },
    "#94A3B8",
    2,
    alpha,
  );
}

function drawBeam(
  ctx: CanvasRenderingContext2D,
  base: Point,
  sky: Point,
  width: number,
  alpha: number,
): void {
  const spread = width;
  ctx.save();
  ctx.globalAlpha = clampAlpha(alpha);
  ctx.fillStyle = "rgba(191, 219, 254, 0.18)";
  ctx.beginPath();
  ctx.moveTo(base.x + 22, base.y - 18);
  ctx.lineTo(sky.x - spread, sky.y + 10);
  ctx.lineTo(sky.x + spread, sky.y - 2);
  ctx.closePath();
  ctx.fill();
  ctx.restore();

  drawDottedLine(
    ctx,
    { x: base.x + 25, y: base.y - 17 },
    sky,
    "#DBEAFE",
    alpha * 0.42,
    18,
  );
}

function drawConstellationStar(
  ctx: CanvasRenderingContext2D,
  point: Point,
  size: number,
  alpha: number,
  color: string,
): void {
  fillCircle(ctx, point.x, point.y, size * 2.3, color, alpha * 0.12);
  fillPixelRect(
    ctx,
    point.x - size / 2,
    point.y - size / 2,
    size,
    size,
    color,
    alpha,
  );
  fillPixelRect(
    ctx,
    point.x - size * 1.4,
    point.y,
    size * 2.8,
    1,
    color,
    alpha * 0.68,
  );
  fillPixelRect(
    ctx,
    point.x,
    point.y - size * 1.4,
    1,
    size * 2.8,
    color,
    alpha * 0.68,
  );
}

function drawStargazingConstellation(args: DrawShowcaseEventArgs): void {
  const {
    ctx,
    run,
    frame,
    width,
    height,
    compact,
    reducedMotion,
    world,
    palette,
  } = args;
  const progress = phaseProgress(run, args.nowMs);
  const alpha = reducedAlpha(eventAlpha(run, progress), reducedMotion);
  if (alpha <= 0) return;

  const timeline = timelineProgress(run, progress);
  const base = {
    x: pctX(width, run.target.x),
    y: pctY(height, run.target.y),
  };
  const skyBaseX = width * lerp(0.39, 0.57, seededUnit(run.seed, 210));
  const sweep = reducedMotion
    ? 0
    : Math.sin(frame / 82 + run.seed) * width * (compact ? 0.025 : 0.045);
  const sky = {
    x: clamp(skyBaseX + sweep, width * 0.22, width * 0.78),
    y:
      height * (compact ? 0.21 : 0.17) +
      (reducedMotion ? 0 : Math.sin(frame / 96) * 3),
  };
  const beamProgress = easeOut(clamp01(timeline / 0.34));
  const skyAlpha =
    alpha * (world.phase === "night" || world.phase === "evening" ? 1 : 0.78);

  fillCircle(
    ctx,
    sky.x,
    sky.y + 7,
    compact ? 72 : 108,
    "#C4B5FD",
    skyAlpha * (reducedMotion ? 0.045 : 0.08),
  );
  if (!reducedMotion) {
    drawBeam(
      ctx,
      base,
      sky,
      lerp(20, compact ? 56 : 78, beamProgress),
      skyAlpha * beamProgress,
    );
  } else {
    drawDottedLine(
      ctx,
      { x: base.x + 25, y: base.y - 17 },
      sky,
      "#DBEAFE",
      skyAlpha * beamProgress * 0.28,
      compact ? 7 : 10,
    );
  }
  drawTelescope(ctx, base, palette, alpha);

  const starCount = reducedMotion ? (compact ? 5 : 6) : compact ? 6 : 9;
  const stars = constellationStars(args, sky, starCount);
  const reveal =
    run.phase === "showcase"
      ? clamp01((progress - 0.08) / 0.56)
      : timeline >= 0.72
        ? 1
        : 0;

  for (let index = 0; index < stars.length - 1; index += 1) {
    const linkReveal = clamp01(reveal * (stars.length - 1) - index);
    if (linkReveal <= 0) continue;
    drawDottedLine(
      ctx,
      stars[index],
      stars[index + 1],
      "#BFDBFE",
      skyAlpha * 0.34 * linkReveal,
      compact ? 8 : 12,
    );
  }

  for (let index = 0; index < stars.length; index += 1) {
    const starReveal = clamp01(reveal * stars.length - index + 1);
    const softPulse = reducedMotion
      ? 0.1
      : Math.sin(frame / 34 + index * 1.9) * 0.16 + 0.16;
    const leadPulse =
      index === 1 || index === stars.length - 2 ? softPulse : softPulse * 0.35;
    const size = lerp(
      2,
      compact ? 3.4 : 4.4,
      seededUnit(run.seed, 300 + index),
    );
    const color = index % 2 === 0 ? "#E0E7FF" : "#FDE68A";
    drawConstellationStar(
      ctx,
      stars[index],
      size + leadPulse * 2,
      skyAlpha * starReveal * (0.7 + leadPulse),
      color,
    );
  }

  if (reveal > 0.45) {
    const labelAlpha = skyAlpha * clamp01((reveal - 0.45) / 0.55) * 0.6;
    const twinkle = reducedMotion ? 0 : Math.sin(frame / 48) * 0.04;
    fillPixelRect(
      ctx,
      sky.x - 18,
      sky.y + 30,
      36,
      2,
      palette.light,
      labelAlpha + twinkle,
    );
    fillPixelRect(
      ctx,
      sky.x - 10,
      sky.y + 36,
      20,
      2,
      "#FDE68A",
      labelAlpha * 0.72,
    );
  }
}

const DRAWERS: Record<BuddyShowcaseKind, ShowcaseDrawer> = {
  memory_firefly_night: drawMemoryFireflyNight,
  stargazing_constellation: drawStargazingConstellation,
};

export function drawShowcaseEvent(args: DrawShowcaseEventArgs): void {
  DRAWERS[args.run.kind](args);
}
