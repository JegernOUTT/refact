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

function finiteOrZero(value: number): number {
  return Number.isFinite(value) ? value : 0;
}

function pctX(width: number, value: number): number {
  return finiteOrZero((width * value) / 100);
}

function pctY(height: number, value: number): number {
  return finiteOrZero((height * value) / 100);
}

function clamp(value: number, min: number, max: number): number {
  if (!Number.isFinite(value)) return min;
  return Math.max(min, Math.min(max, value));
}

function clamp01(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return clamp(value, 0, 1);
}

function clampAlpha(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return clamp01(value);
}

function lerp(from: number, to: number, progress: number): number {
  return finiteOrZero(from + (to - from) * progress);
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
  let value = (finiteOrZero(seed) + Math.imul(salt + 1, 0x9e3779b9)) >>> 0;
  value ^= value >>> 16;
  value = Math.imul(value, 0x85ebca6b) >>> 0;
  value ^= value >>> 13;
  value = Math.imul(value, 0xc2b2ae35) >>> 0;
  value ^= value >>> 16;
  return (value >>> 0) / UINT_MAX;
}

function phaseProgress(run: BuddyShowcaseRun, nowMs?: number): number {
  const duration = BUDDY_SHOWCASE_PHASE_DURATIONS_MS[run.phase];
  const elapsed = finiteOrZero(nowMs ?? Date.now()) - run.phaseStartedAtMs;
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
    Math.round(finiteOrZero(x)),
    Math.round(finiteOrZero(y)),
    Math.max(1, Math.round(finiteOrZero(width))),
    Math.max(1, Math.round(finiteOrZero(height))),
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
  ctx.arc(
    finiteOrZero(x),
    finiteOrZero(y),
    Math.max(0, finiteOrZero(radius)),
    0,
    TAU,
  );
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
  ctx.lineWidth = Math.max(0, finiteOrZero(width));
  ctx.lineCap = "round";
  ctx.beginPath();
  ctx.moveTo(finiteOrZero(from.x), finiteOrZero(from.y));
  ctx.lineTo(finiteOrZero(to.x), finiteOrZero(to.y));
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
  const buddyY = finiteOrZero(args.height) * (args.compact ? 0.62 : 0.66);
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
        local * finiteOrZero(height) * (compact ? 0.23 : 0.32);
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
  const safeWidth = finiteOrZero(width);
  const safeHeight = finiteOrZero(height);
  const xRadius = safeWidth * (compact ? 0.17 : 0.2);
  const yRadius = safeHeight * (compact ? 0.1 : 0.12);
  return Array.from({ length: count }, (_, index) => {
    const angle =
      (index / count) * TAU + seededUnit(run.seed, 100 + index) * 0.72;
    const radius = lerp(0.42, 1, seededUnit(run.seed, 140 + index));
    return {
      x: clamp(
        center.x + Math.cos(angle) * xRadius * radius,
        18,
        safeWidth - 18,
      ),
      y: clamp(
        center.y + Math.sin(angle) * yRadius * radius,
        18,
        safeHeight * 0.48,
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
  const spread = finiteOrZero(width);
  ctx.save();
  ctx.globalAlpha = clampAlpha(alpha);
  ctx.fillStyle = "rgba(191, 219, 254, 0.18)";
  ctx.beginPath();
  ctx.moveTo(finiteOrZero(base.x + 22), finiteOrZero(base.y - 18));
  ctx.lineTo(finiteOrZero(sky.x - spread), finiteOrZero(sky.y + 10));
  ctx.lineTo(finiteOrZero(sky.x + spread), finiteOrZero(sky.y - 2));
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
  const safeWidth = finiteOrZero(width);
  const safeHeight = finiteOrZero(height);
  const safeSeed = finiteOrZero(run.seed);
  const skyBaseX = safeWidth * lerp(0.39, 0.57, seededUnit(run.seed, 210));
  const sweep = reducedMotion
    ? 0
    : Math.sin(frame / 82 + safeSeed) * safeWidth * (compact ? 0.025 : 0.045);
  const sky = {
    x: clamp(skyBaseX + sweep, safeWidth * 0.22, safeWidth * 0.78),
    y:
      safeHeight * (compact ? 0.21 : 0.17) +
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

function fillEllipse(
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
    Math.max(0, finiteOrZero(radiusX)),
    Math.max(0, finiteOrZero(radiusY)),
    0,
    0,
    TAU,
  );
  ctx.fill();
  ctx.restore();
}

function strokeEllipseLocal(
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
  ctx.lineWidth = Math.max(0.5, finiteOrZero(width));
  ctx.beginPath();
  ctx.ellipse(
    finiteOrZero(x),
    finiteOrZero(y),
    Math.max(0, finiteOrZero(radiusX)),
    Math.max(0, finiteOrZero(radiusY)),
    0,
    0,
    TAU,
  );
  ctx.stroke();
  ctx.restore();
}

function drawGlyph(
  ctx: CanvasRenderingContext2D,
  text: string,
  x: number,
  y: number,
  color: string,
  alpha: number,
): void {
  ctx.save();
  ctx.globalAlpha = clampAlpha(alpha);
  ctx.font = "10px monospace";
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.fillStyle = color;
  ctx.fillText(text, finiteOrZero(x), finiteOrZero(y));
  ctx.restore();
}

function buddyStand(args: DrawShowcaseEventArgs): Point {
  return {
    x: pctX(args.width, clamp(args.run.target.x, 33, 67)),
    y: pctY(args.height, args.run.target.y),
  };
}

function drawRainShelterDash(args: DrawShowcaseEventArgs): void {
  const { ctx, run, frame, width, height, compact, reducedMotion } = args;
  const progress = phaseProgress(run, args.nowMs);
  const alpha = reducedAlpha(eventAlpha(run, progress), reducedMotion);
  if (alpha <= 0) return;

  const timeline = timelineProgress(run, progress);
  const stand = buddyStand(args);
  const safeWidth = finiteOrZero(width);
  const streakCount = reducedMotion ? (compact ? 6 : 9) : compact ? 12 : 18;
  const rainBoost = timeline < 0.4 ? 0.6 + timeline : 1;

  for (let index = 0; index < streakCount; index += 1) {
    const rx =
      (seededUnit(run.seed, index * 7) * safeWidth +
        (reducedMotion ? 0 : frame * 1.4)) %
      safeWidth;
    const ry =
      ((seededUnit(run.seed, index * 7 + 1) * finiteOrZero(height) +
        (reducedMotion ? 0 : frame * 3.2)) %
        (finiteOrZero(height) * 0.6)) +
      8;
    fillPixelRect(
      ctx,
      rx,
      ry,
      1.4,
      6.5,
      "#7DD3FC",
      alpha *
        0.4 *
        rainBoost *
        (0.5 + seededUnit(run.seed, index * 7 + 2) * 0.5),
    );
  }

  const awningY = stand.y - (compact ? 34 : 44);
  const awningW = compact ? 26 : 34;
  fillEllipse(ctx, stand.x, awningY, awningW, 7, "#3E7C4F", alpha * 0.9);
  fillEllipse(
    ctx,
    stand.x - 4,
    awningY - 3,
    awningW * 0.7,
    4.4,
    "#5C9450",
    alpha * 0.92,
  );
  fillPixelRect(
    ctx,
    stand.x + awningW * 0.55,
    awningY - 2,
    2,
    5,
    "#79B26A",
    alpha * 0.8,
  );

  const dripCycle = reducedMotion ? 0.5 : ((frame * 1.6) % 34) / 34;
  for (let drip = 0; drip < 3; drip += 1) {
    const t = (dripCycle + drip * 0.33) % 1;
    const dx = stand.x - awningW + drip * awningW;
    const dy = awningY + 6 + t * (stand.y - awningY - 4);
    fillPixelRect(ctx, dx, dy, 1.6, 3, "#BAE6FD", alpha * (1 - t) * 0.8);
    if (t > 0.88) {
      drawSpark(ctx, dx, stand.y + 2, 1.2, "#E0F2FE", alpha * 0.7);
    }
  }

  if (timeline > 0.66) {
    const reveal = clamp01((timeline - 0.66) / 0.34);
    const bands = ["#F87171", "#FACC15", "#4ADE80", "#60A5FA"];
    for (let band = 0; band < bands.length; band += 1) {
      strokeEllipseLocal(
        ctx,
        stand.x + (compact ? 40 : 58),
        stand.y + 6,
        (compact ? 30 : 42) + band * 3.2,
        (compact ? 16 : 22) + band * 2.2,
        bands[band],
        2,
        alpha * reveal * 0.16,
      );
    }
  }
}

function drawKoiPondWatch(args: DrawShowcaseEventArgs): void {
  const { ctx, run, frame, width, height, compact, reducedMotion } = args;
  const progress = phaseProgress(run, args.nowMs);
  const alpha = reducedAlpha(eventAlpha(run, progress), reducedMotion);
  if (alpha <= 0) return;

  const timeline = timelineProgress(run, progress);
  const pond = {
    x: finiteOrZero(width) * 0.13,
    y: finiteOrZero(height) * 0.875,
  };
  const pondRX = compact ? 26 : 38;

  fillEllipse(
    ctx,
    pond.x,
    pond.y,
    pondRX,
    pondRX * 0.26,
    "#0EA5E9",
    alpha * 0.2,
  );

  const jumps = reducedMotion ? 1 : 2;
  for (let koi = 0; koi < jumps; koi += 1) {
    const cycle = reducedMotion
      ? 0.5
      : ((frame * 0.9 + koi * 47 + seededUnit(run.seed, koi) * 40) % 110) / 110;
    if (cycle > 0.62) continue;
    const t = cycle / 0.62;
    const startX = pond.x - pondRX * 0.5 + koi * pondRX * 0.7;
    const arcX = startX + t * pondRX;
    const arcY = pond.y - Math.sin(t * Math.PI) * (compact ? 16 : 24);
    const koiColor = koi === 0 ? "#FB923C" : "#F8FAFC";
    fillPixelRect(ctx, arcX, arcY, 4.4, 2.4, koiColor, alpha * 0.95);
    fillPixelRect(
      ctx,
      arcX - 2.4,
      arcY + (t < 0.5 ? 1.4 : -1.4),
      2.4,
      1.6,
      koiColor,
      alpha * 0.8,
    );
    fillPixelRect(ctx, arcX + 1, arcY + 0.6, 1.4, 1, "#0F172A", alpha * 0.9);
    if (t < 0.16 || t > 0.84) {
      const splashX = t < 0.5 ? startX : startX + pondRX;
      strokeEllipseLocal(
        ctx,
        splashX,
        pond.y + 1,
        5 + (t < 0.5 ? t : 1 - t) * 22,
        2.2,
        "#BAE6FD",
        1.2,
        alpha * 0.6,
      );
      for (let bead = 0; bead < 3; bead += 1) {
        fillPixelRect(
          ctx,
          splashX - 4 + bead * 4,
          pond.y - 4 - bead,
          1.2,
          1.2,
          "#E0F2FE",
          alpha * 0.6,
        );
      }
    }
  }

  const bob = reducedMotion ? 0 : Math.sin(frame / 26) * 1.4;
  fillEllipse(
    ctx,
    pond.x + pondRX * 0.4,
    pond.y - 2 + bob * 0.4,
    6,
    2.4,
    "#16A34A",
    alpha * 0.9,
  );
  fillPixelRect(
    ctx,
    pond.x + pondRX * 0.4,
    pond.y - 4 + bob * 0.4,
    2,
    2,
    "#F9A8D4",
    alpha * 0.92,
  );

  if (timeline > 0.5) {
    drawDottedLine(
      ctx,
      { x: buddyStand(args).x - 8, y: buddyStand(args).y - 14 },
      { x: pond.x + 8, y: pond.y - 10 },
      "#BAE6FD",
      alpha * clamp01((timeline - 0.5) / 0.5) * 0.3,
      compact ? 6 : 9,
    );
  }
}

function drawCampfireStory(args: DrawShowcaseEventArgs): void {
  const { ctx, run, frame, compact, reducedMotion } = args;
  const progress = phaseProgress(run, args.nowMs);
  const alpha = reducedAlpha(eventAlpha(run, progress), reducedMotion);
  if (alpha <= 0) return;

  const timeline = timelineProgress(run, progress);
  const stand = buddyStand(args);
  const fire = { x: stand.x + (compact ? 18 : 26), y: stand.y + 4 };
  const flare = 0.6 + timeline * 0.5;

  fillCircle(
    ctx,
    fire.x,
    fire.y - 6,
    (compact ? 16 : 22) * flare,
    "#FB923C",
    alpha * 0.16,
  );
  for (let tongue = 0; tongue < 3; tongue += 1) {
    const sway = reducedMotion ? 0 : Math.sin(frame / 7 + tongue * 1.8) * 2.2;
    const h = (7 + tongue * 3.4) * flare;
    fillPixelRect(
      ctx,
      fire.x - 2.4 + tongue * 1.8 + sway,
      fire.y - h,
      2.6,
      h,
      tongue === 1 ? "#FDE68A" : "#FB923C",
      alpha * (0.86 - tongue * 0.16),
    );
  }
  fillPixelRect(ctx, fire.x - 6, fire.y, 12, 2.4, "#7C2D12", alpha * 0.9);

  const sparkCount = reducedMotion ? 3 : compact ? 5 : 8;
  for (let index = 0; index < sparkCount; index += 1) {
    const rise = ((frame * 1.1 + index * 23) % 70) / 70;
    const sx =
      fire.x +
      (reducedMotion ? 0 : Math.sin(frame / 9 + index * 2.2) * (3 + rise * 7));
    const sy = fire.y - 8 - rise * (compact ? 30 : 44);
    fillPixelRect(
      ctx,
      sx,
      sy,
      1.4,
      1.4,
      index % 2 === 0 ? "#FDE68A" : "#FCA5A5",
      alpha * (1 - rise) * 0.85,
    );
  }

  if (!reducedMotion) {
    for (let puff = 0; puff < 2; puff += 1) {
      const rise = ((frame * 0.55 + puff * 42) % 90) / 90;
      fillCircle(
        ctx,
        fire.x + 4 + Math.sin(frame / 22 + puff * 2) * 4 + rise * 7,
        fire.y - 24 - rise * 30,
        2.6 + rise * 4.4,
        "#94A3B8",
        alpha * (1 - rise) * 0.2,
      );
    }
  }

  if (run.phase === "showcase") {
    const storyReveal = clamp01((progress - 0.2) / 0.6);
    const glyphs = ["★", "♥", "♪"];
    for (let g = 0; g < glyphs.length; g += 1) {
      const gReveal = clamp01(storyReveal * 3 - g);
      if (gReveal <= 0) continue;
      const drift = reducedMotion ? 0 : Math.sin(frame / 18 + g * 2.4) * 3;
      drawGlyph(
        ctx,
        glyphs[g],
        fire.x + 10 + g * 11 + drift,
        fire.y - 38 - g * 9 - gReveal * 7,
        g === 1 ? "#F9A8D4" : "#FDE68A",
        alpha * gReveal * 0.84,
      );
    }
  }
}

function drawFireflyMeadowChase(args: DrawShowcaseEventArgs): void {
  const { ctx, run, frame, width, height, compact, reducedMotion } = args;
  const progress = phaseProgress(run, args.nowMs);
  const alpha = reducedAlpha(eventAlpha(run, progress), reducedMotion);
  if (alpha <= 0) return;

  const timeline = timelineProgress(run, progress);
  const stand = buddyStand(args);
  const count = reducedMotion ? (compact ? 6 : 9) : compact ? 12 : 18;
  const spread = compact ? 44 : 66;

  for (let index = 0; index < count; index += 1) {
    const baseAngle = seededUnit(run.seed, index * 13) * TAU;
    const radius = lerp(10, spread, seededUnit(run.seed, index * 13 + 1));
    const wobble = reducedMotion
      ? 0
      : Math.sin(frame / (9 + (index % 5)) + index * 1.7) * 6;
    let x: number;
    let y: number;
    let glow = alpha * lerp(0.5, 0.95, seededUnit(run.seed, index * 13 + 2));

    if (timeline < 0.3) {
      const local = easeInOut(clamp01(timeline / 0.3));
      const fromX = seededUnit(run.seed, index * 13 + 3) * finiteOrZero(width);
      const fromY =
        finiteOrZero(height) *
        (0.3 + seededUnit(run.seed, index * 13 + 4) * 0.4);
      x = lerp(fromX, stand.x + Math.cos(baseAngle) * radius, local);
      y = lerp(fromY, stand.y - 14 + Math.sin(baseAngle) * radius * 0.4, local);
    } else if (timeline < 0.72) {
      const local = (timeline - 0.3) / 0.42;
      const zig = Math.floor(
        local * 4 + seededUnit(run.seed, index * 13 + 5) * 2,
      );
      const zigSeedX = seededUnit(run.seed, index * 17 + zig * 3) - 0.5;
      const zigSeedY = seededUnit(run.seed, index * 17 + zig * 3 + 1) - 0.5;
      x = stand.x + zigSeedX * spread * 2 + wobble;
      y = stand.y - 16 + zigSeedY * spread * 0.7 + wobble * 0.4;
      glow *=
        0.85 +
        (reducedMotion ? 0 : Math.abs(Math.sin(frame / 5 + index)) * 0.3);
    } else {
      const local = easeOut(clamp01((timeline - 0.72) / 0.28));
      const angle = baseAngle + local * TAU * (reducedMotion ? 0.3 : 1.1);
      const ringRadius = lerp(spread * 0.9, compact ? 16 : 22, local);
      x = stand.x + Math.cos(angle) * ringRadius + wobble * 0.4;
      y = stand.y - 16 + Math.sin(angle) * ringRadius * 0.42;
    }

    const color =
      index % 3 === 0 ? "#FEF3C7" : index % 3 === 1 ? "#FDE68A" : "#A7F3D0";
    drawSpark(
      ctx,
      x,
      y,
      1.4 + seededUnit(run.seed, index * 13 + 6) * 1.6,
      color,
      glow,
    );
  }

  if (timeline >= 0.3 && timeline < 0.72 && !reducedMotion) {
    const dashPhase = (frame % 26) / 26;
    if (dashPhase < 0.4) {
      const dir = Math.sin(frame / 26) > 0 ? 1 : -1;
      for (let streak = 0; streak < 3; streak += 1) {
        fillPixelRect(
          ctx,
          stand.x - dir * (8 + streak * 7 + dashPhase * 18),
          stand.y - 6 + streak * 3,
          6 - streak,
          1.2,
          "#E2E8F0",
          alpha * (1 - dashPhase) * 0.34,
        );
      }
    }
  }
}

function drawSnowSculpting(args: DrawShowcaseEventArgs): void {
  const { ctx, run, frame, compact, reducedMotion } = args;
  const progress = phaseProgress(run, args.nowMs);
  const alpha = reducedAlpha(eventAlpha(run, progress), reducedMotion);
  if (alpha <= 0) return;

  const timeline = timelineProgress(run, progress);
  const stand = buddyStand(args);
  const mound = { x: stand.x + (compact ? 16 : 22), y: stand.y + 6 };
  const build = clamp01(timeline / 0.72);
  const stage = build < 0.34 ? 0 : build < 0.7 ? 1 : 2;

  fillEllipse(ctx, mound.x, mound.y + 2, 12, 3.2, "#E0F2FE", alpha * 0.5);
  fillEllipse(
    ctx,
    mound.x,
    mound.y - 2,
    9,
    6,
    "#F8FAFC",
    alpha * (0.5 + build * 0.45),
  );
  if (stage >= 1) {
    fillEllipse(ctx, mound.x, mound.y - 9, 6.4, 4.6, "#F8FAFC", alpha * 0.92);
  }
  if (stage >= 2) {
    fillEllipse(
      ctx,
      mound.x,
      mound.y - 15.4,
      4.4,
      3.6,
      "#F8FAFC",
      alpha * 0.95,
    );
    fillPixelRect(
      ctx,
      mound.x - 1.8,
      mound.y - 16.4,
      1.2,
      1.2,
      "#0F172A",
      alpha,
    );
    fillPixelRect(
      ctx,
      mound.x + 0.8,
      mound.y - 16.4,
      1.2,
      1.2,
      "#0F172A",
      alpha,
    );
    fillPixelRect(
      ctx,
      mound.x - 1,
      mound.y - 14,
      2.4,
      0.9,
      "#0F172A",
      alpha * 0.9,
    );
    fillPixelRect(
      ctx,
      mound.x - 4.4,
      mound.y - 10,
      3,
      1,
      "#A16207",
      alpha * 0.9,
    );
    fillPixelRect(
      ctx,
      mound.x + 1.6,
      mound.y - 10,
      3,
      1,
      "#A16207",
      alpha * 0.9,
    );
  }

  if (timeline < 0.72 && !reducedMotion) {
    const burst = (frame % 18) / 18;
    for (let fleck = 0; fleck < 5; fleck += 1) {
      const angle = Math.PI * (0.8 + fleck * 0.14);
      fillPixelRect(
        ctx,
        stand.x + 6 + Math.cos(angle) * (4 + burst * 12),
        stand.y - 2 - Math.sin(burst * Math.PI) * (6 + fleck),
        1.4,
        1.4,
        "#F8FAFC",
        alpha * (1 - burst) * 0.8,
      );
    }
  }

  if (stage >= 2) {
    const sparkle = reducedMotion ? 0.4 : Math.abs(Math.sin(frame / 14));
    drawSpark(
      ctx,
      mound.x + 7,
      mound.y - 18,
      1.6,
      "#BAE6FD",
      alpha * sparkle * 0.9,
    );
  }
}

function drawLeafStormPlay(args: DrawShowcaseEventArgs): void {
  const { ctx, run, frame, compact, reducedMotion } = args;
  const progress = phaseProgress(run, args.nowMs);
  const alpha = reducedAlpha(eventAlpha(run, progress), reducedMotion);
  if (alpha <= 0) return;

  const timeline = timelineProgress(run, progress);
  const stand = buddyStand(args);
  const count = reducedMotion ? (compact ? 7 : 10) : compact ? 12 : 16;
  const colors = ["#FB923C", "#D97706", "#F87171", "#FACC15"];
  const vortexHeight = compact ? 38 : 54;
  const settle =
    timeline > 0.74 ? easeOut(clamp01((timeline - 0.74) / 0.26)) : 0;

  for (let index = 0; index < count; index += 1) {
    const phase01 =
      (seededUnit(run.seed, index * 9) + (reducedMotion ? 0 : frame / 90)) % 1;
    const spiralAngle = phase01 * TAU * 2 + index;
    const spiralRadius = lerp(6, compact ? 24 : 34, phase01);
    const lift = phase01 * vortexHeight;
    const x = stand.x + Math.cos(spiralAngle) * spiralRadius;
    const yVortex = stand.y + 2 - lift;
    const yGround = stand.y + 4 + seededUnit(run.seed, index * 9 + 1) * 5;
    const y = lerp(yVortex, yGround, settle);
    const flutter = reducedMotion ? 0 : Math.sin(frame / 6 + index * 2.1);
    fillPixelRect(
      ctx,
      x,
      y,
      2.4 + flutter * 0.7,
      1.8,
      colors[index % colors.length],
      alpha *
        (0.55 + seededUnit(run.seed, index * 9 + 2) * 0.4) *
        (1 - settle * 0.4),
    );
  }

  if (timeline >= 0.3 && timeline < 0.74 && !reducedMotion) {
    const hop = Math.abs(Math.sin(frame / 11));
    if (hop > 0.72) {
      fillEllipse(ctx, stand.x, stand.y + 8, 8, 2, "#D6CDBF", alpha * 0.4);
    }
  }
}

function drawAuroraDance(args: DrawShowcaseEventArgs): void {
  const { ctx, run, frame, width, height, compact, reducedMotion } = args;
  const progress = phaseProgress(run, args.nowMs);
  const alpha = reducedAlpha(eventAlpha(run, progress), reducedMotion);
  if (alpha <= 0) return;

  const timeline = timelineProgress(run, progress);
  const stand = buddyStand(args);
  const safeWidth = finiteOrZero(width);
  const skyY = finiteOrZero(height) * 0.2;
  const ribbons = ["#2DD4BF", "#A855F7", "#60A5FA"];
  const intensify = 0.4 + clamp01(timeline / 0.6) * 0.6;

  for (let ribbon = 0; ribbon < ribbons.length; ribbon += 1) {
    const sway = reducedMotion
      ? 0
      : Math.sin(frame / (34 + ribbon * 9) + ribbon * 2) * 9;
    ctx.save();
    ctx.globalAlpha = clampAlpha(alpha * 0.3 * intensify);
    ctx.strokeStyle = ribbons[ribbon];
    ctx.lineWidth = compact ? 4 : 6;
    ctx.lineCap = "round";
    ctx.beginPath();
    ctx.moveTo(0, skyY + ribbon * 12 + sway);
    ctx.bezierCurveTo(
      safeWidth * 0.3,
      skyY - 16 + ribbon * 9 + sway,
      safeWidth * 0.62,
      skyY + 22 - ribbon * 7 - sway,
      safeWidth,
      skyY - 4 + ribbon * 10,
    );
    ctx.stroke();
    ctx.restore();
  }

  fillEllipse(
    ctx,
    stand.x,
    stand.y + 7,
    compact ? 22 : 32,
    4.4,
    ribbons[Math.floor(timeline * 2.99) % 3],
    alpha * 0.2 * intensify,
  );

  if (run.phase === "showcase" && !reducedMotion) {
    const ringCount = compact ? 5 : 7;
    for (let index = 0; index < ringCount; index += 1) {
      const angle = frame / 16 + index * (TAU / ringCount);
      const radius = 14 + Math.sin(frame / 12 + index) * 4;
      drawSpark(
        ctx,
        stand.x + Math.cos(angle) * radius,
        stand.y - 12 + Math.sin(angle) * radius * 0.5,
        1.4,
        ribbons[index % 3],
        alpha * 0.7,
      );
    }
  }
}

function drawKomorebiNap(args: DrawShowcaseEventArgs): void {
  const { ctx, run, frame, width, height, compact, reducedMotion } = args;
  const progress = phaseProgress(run, args.nowMs);
  const alpha = reducedAlpha(eventAlpha(run, progress), reducedMotion);
  if (alpha <= 0) return;

  const timeline = timelineProgress(run, progress);
  const stand = buddyStand(args);
  const canopyX = finiteOrZero(width) * 0.3;
  const canopyY = finiteOrZero(height) * 0.36;

  for (let beam = 0; beam < (compact ? 2 : 3); beam += 1) {
    const sway = reducedMotion ? 0 : Math.sin(frame / 60 + beam * 1.8) * 5;
    const topX = canopyX + beam * 16 - 10 + sway;
    const bottomX = stand.x - 12 + beam * 14 + sway * 0.5;
    ctx.save();
    ctx.globalAlpha = clampAlpha(alpha * 0.12);
    ctx.fillStyle = "#FDE68A";
    ctx.beginPath();
    ctx.moveTo(topX, canopyY);
    ctx.lineTo(topX + 7, canopyY);
    ctx.lineTo(bottomX + 13, stand.y + 9);
    ctx.lineTo(bottomX, stand.y + 9);
    ctx.closePath();
    ctx.fill();
    ctx.restore();
  }

  const spotCount = reducedMotion ? 4 : compact ? 6 : 9;
  for (let index = 0; index < spotCount; index += 1) {
    const shimmer = reducedMotion
      ? 0.5
      : 0.3 + Math.abs(Math.sin(frame / 30 + index * 1.9)) * 0.7;
    fillEllipse(
      ctx,
      stand.x - 22 + seededUnit(run.seed, index * 5) * 48,
      stand.y + 6 + seededUnit(run.seed, index * 5 + 1) * 6,
      2.4 + seededUnit(run.seed, index * 5 + 2) * 3,
      1.2,
      "#FEF3C7",
      alpha * shimmer * 0.4,
    );
  }

  if (timeline > 0.3) {
    for (let z = 0; z < 3; z += 1) {
      const rise = ((frame * 0.45 + z * 26) % 80) / 80;
      drawGlyph(
        ctx,
        "z",
        stand.x + 10 + rise * 9 + z * 3,
        stand.y - 20 - rise * 20 - z * 2,
        "#C7D2FE",
        alpha * (1 - rise) * 0.7,
      );
    }
  }

  if (!reducedMotion) {
    const fall = ((frame * 0.5) % 110) / 110;
    fillPixelRect(
      ctx,
      canopyX + 14 + Math.sin(frame / 19) * 8,
      canopyY + fall * (stand.y - canopyY),
      2.4,
      1.8,
      "#86EFAC",
      alpha * (1 - fall) * 0.7,
    );
  }
}

const DRAWERS: Record<BuddyShowcaseKind, ShowcaseDrawer> = {
  memory_firefly_night: drawMemoryFireflyNight,
  stargazing_constellation: drawStargazingConstellation,
  rain_shelter_dash: drawRainShelterDash,
  koi_pond_watch: drawKoiPondWatch,
  campfire_story: drawCampfireStory,
  firefly_meadow_chase: drawFireflyMeadowChase,
  snow_sculpting: drawSnowSculpting,
  leaf_storm_play: drawLeafStormPlay,
  aurora_dance: drawAuroraDance,
  komorebi_nap: drawKomorebiNap,
};

export function drawShowcaseEvent(args: DrawShowcaseEventArgs): void {
  DRAWERS[args.run.kind](args);
}
