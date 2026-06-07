import type { BuddyWorldObject } from "./buddyWorldModel";
import {
  BUDDY_WORLD_HOME_HOTSPOT,
  alphaForMotion,
  drawPixelText,
  drawSpark,
  fillCircle,
  fillEllipse,
  fillPixelRect,
  finiteOr,
  pctX,
  pctY,
  safeDimension,
  safeFrame,
  strokeLine,
  strokeEllipse,
  toneColor,
  wave,
  worldObjects,
  type DrawBuddyWorldBaseArgs,
} from "./buddyWorldDrawHelpers";

function objectPulse(
  args: DrawBuddyWorldBaseArgs,
  item: BuddyWorldObject,
): number {
  if (args.reducedMotion) return 0;
  return (
    Math.sin(safeFrame(args.frame) / 24 + finiteOr(item.x, 0)) *
    2 *
    (0.7 + finiteOr(item.intensity, 0) * 0.3)
  );
}

function objectAlpha(
  args: DrawBuddyWorldBaseArgs,
  item: BuddyWorldObject,
): number {
  const base =
    item.state === "critical" ? 0.18 : item.state === "active" ? 0.12 : 0.08;
  return alphaForMotion(
    base + finiteOr(item.intensity, 0) * 0.08,
    args.reducedMotion,
  );
}

export function drawBuddyHomeDoor(args: DrawBuddyWorldBaseArgs): void {
  const { ctx, palette } = args;
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const frame = safeFrame(args.frame);
  const x = pctX(width, BUDDY_WORLD_HOME_HOTSPOT.x);
  const y = pctY(height, BUDDY_WORLD_HOME_HOTSPOT.y);
  const glow = 0.28 + wave(frame, 32, 0, 0.08, args.reducedMotion);
  const scale = args.compact ? 0.86 : 1;

  fillEllipse(ctx, x, y + 14 * scale, 35 * scale, 15 * scale, "#FBBF24", glow);

  const pathGlow = 0.36 + wave(frame, 40, 0, 0.04, args.reducedMotion);
  for (let index = 0; index < 6; index += 1) {
    const stepX = x + index * 9 * scale + Math.sin(index * 1.7) * 4;
    const stepY = y + 32 * scale + index * 5 * scale;
    fillEllipse(
      ctx,
      stepX,
      stepY,
      8 - index * 0.45,
      3.4,
      "#B45309",
      pathGlow - index * 0.035,
    );
  }

  fillPixelRect(
    ctx,
    x - 23 * scale,
    y - 1 * scale,
    46 * scale,
    30 * scale,
    "#92400E",
  );
  fillPixelRect(
    ctx,
    x - 28 * scale,
    y + 25 * scale,
    56 * scale,
    5 * scale,
    "#0F172A",
    0.36,
  );
  fillPixelRect(
    ctx,
    x - 17 * scale,
    y - 15 * scale,
    34 * scale,
    8 * scale,
    palette.dark,
  );
  fillPixelRect(
    ctx,
    x - 22 * scale,
    y - 7 * scale,
    44 * scale,
    8 * scale,
    palette.dark,
  );
  fillPixelRect(
    ctx,
    x - 14 * scale,
    y - 29 * scale,
    7 * scale,
    12 * scale,
    "#475569",
  );
  fillPixelRect(
    ctx,
    x - 7 * scale,
    y + 6 * scale,
    14 * scale,
    23 * scale,
    "#1E293B",
  );
  fillPixelRect(
    ctx,
    x - 4 * scale,
    y + 10 * scale,
    8 * scale,
    19 * scale,
    "#0F172A",
  );
  fillPixelRect(
    ctx,
    x + 7 * scale,
    y + 7 * scale,
    8 * scale,
    8 * scale,
    "#FDE68A",
  );
  fillPixelRect(
    ctx,
    x + 9 * scale,
    y + 9 * scale,
    4 * scale,
    4 * scale,
    palette.light,
  );
  fillPixelRect(
    ctx,
    x - 12 * scale,
    y + 31 * scale,
    24 * scale,
    3 * scale,
    "#FBBF24",
  );

  fillPixelRect(
    ctx,
    x - 26 * scale,
    y - 43 * scale,
    52 * scale,
    12 * scale,
    "#0F172A",
    0.86,
  );
  fillPixelRect(
    ctx,
    x - 23 * scale,
    y - 40 * scale,
    46 * scale,
    2 * scale,
    palette.body,
  );
  if (!args.compact)
    drawPixelText(ctx, "HOME", x, y - 36 * scale, palette.light);
  fillPixelRect(
    ctx,
    x - 2 * scale,
    y - 31 * scale,
    4 * scale,
    7 * scale,
    palette.body,
  );

  const sparkleY = y - 7 * scale + wave(frame, 18, 0, 2, args.reducedMotion);
  drawSpark(
    ctx,
    x + 30 * scale,
    sparkleY + 2 * scale,
    1.8 * scale,
    "#FDE68A",
    0.82,
  );
}

function drawTaskGrove(
  args: DrawBuddyWorldBaseArgs,
  item: BuddyWorldObject,
  x: number,
  y: number,
  pulse: number,
  tone: string,
): void {
  fillPixelRect(args.ctx, x - 5, y - 4, 10, 32, "#7C2D12");
  fillPixelRect(
    args.ctx,
    x - 17,
    y - 22 + pulse,
    34,
    18,
    item.state === "critical" ? "#84CC16" : "#22C55E",
  );
  fillPixelRect(args.ctx, x - 10, y - 31 + pulse, 22, 14, "#86EFAC");
  fillPixelRect(args.ctx, x + 11, y - 11 + pulse, 9, 7, "#BBF7D0");
  fillPixelRect(args.ctx, x + 14, y - 8 + pulse, 6, 3, tone);
}

function drawMemoryFireflies(
  args: DrawBuddyWorldBaseArgs,
  item: BuddyWorldObject,
  x: number,
  y: number,
  tone: string,
): void {
  const count = args.reducedMotion ? 4 : args.compact ? 5 : 7;
  const attention = item.state === "attention" || item.state === "critical";
  const active = item.state === "active" || item.animation === "stream";
  const glowColor =
    item.state === "critical" ? "#EF4444" : attention ? "#F59E0B" : "#FDE68A";

  fillCircle(
    args.ctx,
    x,
    y + 15,
    item.state === "critical" ? 28 : 22,
    glowColor,
    attention ? 0.12 : 0.08,
  );
  for (let index = 0; index < count; index += 1) {
    const fx =
      x +
      wave(
        args.frame,
        active ? 12 : 18,
        index,
        8 + index * 2,
        args.reducedMotion,
      );
    const fy =
      y +
      Math.cos(safeFrame(args.frame) / 15 + index) *
        (args.reducedMotion ? 0 : active ? 18 : 12);
    drawSpark(
      args.ctx,
      fx,
      fy,
      1.8,
      index % 2 === 0 ? glowColor : tone,
      0.62 + finiteOr(item.intensity, 0) * 0.2,
    );
    if (active && index % 2 === 0) {
      strokeLine(
        args.ctx,
        { x: fx, y: fy },
        { x: x + 72, y: y + 42 - index * 3 },
        "#FDE68A",
        1.4,
        0.16 + finiteOr(item.intensity, 0) * 0.12,
      );
    }
  }
  fillPixelRect(args.ctx, x - 14, y + 15, 28, 11, "#854D0E");
  fillPixelRect(args.ctx, x - 9, y + 10, 18, 6, glowColor);
  fillPixelRect(args.ctx, x - 18, y + 24, 36, 4, "#422006", 0.46);
}

function drawObservatory(
  args: DrawBuddyWorldBaseArgs,
  item: BuddyWorldObject,
  x: number,
  y: number,
  tone: string,
): void {
  const activeAlpha =
    item.state === "critical" ? 0.32 : item.state === "active" ? 0.2 : 0.1;
  const warning = item.state === "attention";
  fillCircle(args.ctx, x + 11, y - 23, 25, tone, activeAlpha);
  fillPixelRect(args.ctx, x - 24, y + 13, 48, 18, "#334155");
  fillPixelRect(args.ctx, x - 18, y + 4, 36, 15, "#64748B");
  fillPixelRect(args.ctx, x - 10, y - 3, 20, 8, "#94A3B8");
  fillPixelRect(args.ctx, x - 4, y - 19, 8, 18, tone);
  fillPixelRect(args.ctx, x + 4, y - 14, 26, 6, "#CBD5E1");
  fillPixelRect(args.ctx, x + 27, y - 15, 5, 8, "#FDE68A");
  if (item.state === "active") {
    strokeLine(
      args.ctx,
      { x: x + 31, y: y - 16 },
      { x: x - 48, y: y - 50 + wave(args.frame, 58, 0, 8, args.reducedMotion) },
      "#DBEAFE",
      3,
      0.26 + finiteOr(item.intensity, 0) * 0.18,
    );
  }
  if (warning) {
    strokeEllipse(
      args.ctx,
      x + 7,
      y - 11,
      34,
      18,
      "#F59E0B",
      2,
      0.14 + finiteOr(item.intensity, 0) * 0.08,
    );
  }
  if (item.state === "critical") {
    fillCircle(args.ctx, x + 12, y - 24, 32, "#EF4444", 0.13);
    fillPixelRect(args.ctx, x + 33, y - 18, 8, 3, "#FACC15", 0.86);
    fillPixelRect(args.ctx, x + 38, y - 15, 3, 8, "#FACC15", 0.86);
    strokeLine(
      args.ctx,
      { x: x + 34, y: y - 17 },
      { x: x + 58, y: y - 44 },
      "#FACC15",
      2,
      0.76,
    );
  }
}

function drawSatellite(
  args: DrawBuddyWorldBaseArgs,
  item: BuddyWorldObject,
  x: number,
  y: number,
  pulse: number,
  tone: string,
): void {
  fillPixelRect(args.ctx, x - 8, y - 5 + pulse, 16, 10, "#CBD5E1");
  fillPixelRect(args.ctx, x - 26, y - 3 + pulse, 14, 6, tone);
  fillPixelRect(args.ctx, x + 12, y - 3 + pulse, 14, 6, tone);
  fillPixelRect(args.ctx, x - 1, y + 5 + pulse, 2, 18, "#94A3B8");
  if (item.animation === "orbit") {
    strokeEllipse(
      args.ctx,
      x,
      y + 1 + pulse,
      34,
      9,
      "#DBEAFE",
      1,
      0.12 + finiteOr(item.intensity, 0) * 0.08,
    );
  }
}

function drawGitVane(
  args: DrawBuddyWorldBaseArgs,
  x: number,
  y: number,
  tone: string,
): void {
  fillPixelRect(args.ctx, x - 2, y - 18, 4, 42, "#94A3B8");
  fillPixelRect(args.ctx, x - 14, y - 9, 28, 3, "#CBD5E1");
  fillPixelRect(args.ctx, x - 1, y - 22, 3, 30, "#CBD5E1");
  fillPixelRect(args.ctx, x - 18, y - 13, 8, 8, tone);
  fillPixelRect(args.ctx, x + 10, y - 13, 8, 8, "#86EFAC");
  fillPixelRect(args.ctx, x - 5, y - 26, 8, 8, "#F8FAFC");
  fillPixelRect(args.ctx, x - 4, y + 4, 8, 8, "#FDE68A");
}

function drawMarketComet(
  args: DrawBuddyWorldBaseArgs,
  x: number,
  y: number,
  pulse: number,
): void {
  fillPixelRect(args.ctx, x - 10, y - 7 + pulse, 20, 14, "#A855F7");
  fillPixelRect(args.ctx, x - 5, y - 3 + pulse, 10, 7, "#FDE68A");
  fillPixelRect(args.ctx, x - 29, y + pulse, 17, 3, "#FDBA74", 0.52);
  fillPixelRect(args.ctx, x - 40, y + 3 + pulse, 9, 2, "#FDBA74", 0.32);
}

function drawSeed(args: DrawBuddyWorldBaseArgs, x: number, y: number): void {
  fillPixelRect(args.ctx, x - 3, y, 6, 20, "#15803D");
  fillPixelRect(args.ctx, x - 15, y - 12, 14, 10, "#22C55E");
  fillPixelRect(args.ctx, x + 1, y - 16, 15, 10, "#86EFAC");
}

export function drawWorldObject(
  args: DrawBuddyWorldBaseArgs,
  item: BuddyWorldObject,
): void {
  const x = pctX(args.width, item.x);
  const y = pctY(args.height, item.y);
  const tone = toneColor(item.tone, args.tokenPalette);
  const pulse = objectPulse(args, item);
  const scale = Math.max(0.1, finiteOr(item.depthScale, 1));
  const size = Math.max(1, finiteOr(item.size, 12) * scale);

  fillCircle(
    args.ctx,
    x,
    y + 12 * scale,
    item.state === "critical" ? size + 8 : size + 4,
    tone,
    objectAlpha(args, item),
  );
  if (item.state === "critical" || item.state === "active") {
    strokeEllipse(
      args.ctx,
      x,
      y + size + 10,
      size + 6,
      5,
      tone,
      item.state === "critical" ? 2 : 1,
      item.state === "critical" ? 0.34 : 0.16,
    );
  }

  switch (item.sprite) {
    case "task_grove":
      drawTaskGrove(args, item, x, y, pulse, tone);
      break;
    case "memory_fireflies":
      drawMemoryFireflies(args, item, x, y, tone);
      break;
    case "observatory":
      drawObservatory(args, item, x, y, tone);
      break;
    case "satellite":
      drawSatellite(args, item, x, y, pulse, tone);
      break;
    case "git_vane":
      drawGitVane(args, x, y, tone);
      break;
    case "market_comet":
      drawMarketComet(args, x, y, pulse);
      break;
    case "seed":
      drawSeed(args, x, y);
      break;
  }

  const glint = 0.38 + wave(args.frame, 20, item.x, 0.18, args.reducedMotion);
  drawSpark(
    args.ctx,
    x + size + 7,
    y - size + 3 + pulse,
    1.7,
    "#FDE047",
    glint,
  );
}

export function drawWorldObjects(args: DrawBuddyWorldBaseArgs): void {
  for (const item of worldObjects(args.world)) {
    drawWorldObject(args, item);
  }
}
