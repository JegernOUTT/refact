import type { BuddyWorldCompanion } from "./buddyCompanions";
import { companionPositionAt } from "./buddyCompanions";
import {
  BUDDY_WORLD_HOME_HOTSPOT,
  fillCircle,
  fillEllipse,
  fillPixelRect,
  finiteOr,
  pctX,
  pctY,
  safeDimension,
  safeFrame,
  seededUnit,
  strokeEllipse,
  strokeLine,
  wave,
  type DrawBuddyWorldBaseArgs,
} from "./buddyWorldDrawHelpers";

const SHIRO_BODY = "#E7E5E4";
const SHIRO_BELLY = "#FAFAF9";
const SHIRO_EDGE = "#A8A29E";
const SHIRO_INK = "#44403C";
const SOOT_BODY = "#1C1917";
const SOOT_EYE = "#F8FAFC";
const KURO_BODY = "#1E293B";
const KURO_BEAK = "#FB923C";

function drawShiro(
  args: DrawBuddyWorldBaseArgs,
  companion: BuddyWorldCompanion,
  nowMs: number,
): void {
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const frame = safeFrame(args.frame);
  const position = companionPositionAt(companion, nowMs);
  const x = pctX(width, position.x);
  const y = pctY(height, position.y);
  const k =
    Math.max(0.2, finiteOr(companion.scale, 0.38)) * (args.compact ? 0.84 : 1);
  const moving =
    nowMs - companion.moveStartMs < companion.moveDurationMs &&
    Math.abs(companion.toX - companion.fromX) > 0.5;
  const hop = moving
    ? Math.abs(wave(frame, 3.2, companion.seed, 2.6 * k, args.reducedMotion))
    : 0;
  const bob = moving
    ? 0
    : wave(frame, 26, companion.seed, 1.1 * k, args.reducedMotion);
  const sleeping = companion.pose === "sleep";
  const sitting = companion.pose === "sit";
  const pouncing = companion.pose === "pounce";
  const peeking = companion.pose === "peek";
  const bodyW = 13 * k * (sleeping ? 1.28 : pouncing ? 1.12 : 1);
  const bodyH = 16 * k * (sleeping ? 0.62 : sitting ? 0.9 : peeking ? 0.74 : 1);
  const baseY = y - hop + bob;
  const facing = companion.facing;

  fillEllipse(args.ctx, x, y + 1.5, bodyW * 0.92, 2.6 * k, "#1A2E20", 0.24);
  fillEllipse(
    args.ctx,
    x,
    baseY - bodyH * 0.52,
    bodyW,
    bodyH,
    SHIRO_BODY,
    0.97,
  );
  strokeEllipse(
    args.ctx,
    x,
    baseY - bodyH * 0.52,
    bodyW,
    bodyH,
    SHIRO_EDGE,
    0.8,
    0.55,
  );
  fillEllipse(
    args.ctx,
    x + facing * 0.6,
    baseY - bodyH * 0.4,
    bodyW * 0.66,
    bodyH * 0.62,
    SHIRO_BELLY,
    0.95,
  );

  const earTilt = wave(
    frame,
    34,
    companion.seed + 2,
    0.8 * k,
    args.reducedMotion,
  );
  fillEllipse(
    args.ctx,
    x - bodyW * 0.42,
    baseY - bodyH * 1.18 + earTilt,
    2.1 * k,
    4.4 * k,
    SHIRO_BODY,
    0.97,
  );
  fillEllipse(
    args.ctx,
    x + bodyW * 0.42,
    baseY - bodyH * 1.18 - earTilt,
    2.1 * k,
    4.4 * k,
    SHIRO_BODY,
    0.97,
  );

  const eyeY = baseY - bodyH * 0.78;
  if (sleeping) {
    strokeLine(
      args.ctx,
      { x: x - 3.4 * k + facing, y: eyeY },
      { x: x - 1.2 * k + facing, y: eyeY },
      SHIRO_INK,
      0.9,
      0.85,
    );
    strokeLine(
      args.ctx,
      { x: x + 1.2 * k + facing, y: eyeY },
      { x: x + 3.4 * k + facing, y: eyeY },
      SHIRO_INK,
      0.9,
      0.85,
    );
  } else {
    fillCircle(
      args.ctx,
      x - 2.6 * k + facing * 0.8,
      eyeY,
      1 * k,
      SHIRO_INK,
      0.92,
    );
    fillCircle(
      args.ctx,
      x + 2.6 * k + facing * 0.8,
      eyeY,
      1 * k,
      SHIRO_INK,
      0.92,
    );
  }
  fillPixelRect(
    args.ctx,
    x + facing * 0.8 - 0.7 * k,
    eyeY + 1.7 * k,
    1.4 * k,
    1 * k,
    SHIRO_INK,
    0.8,
  );

  for (let index = 0; index < 3; index += 1) {
    const chevronX = x + facing * 0.6 + (index - 1) * 3 * k;
    const chevronY = baseY - bodyH * 0.42;
    strokeLine(
      args.ctx,
      { x: chevronX - 1.1 * k, y: chevronY + 0.9 * k },
      { x: chevronX, y: chevronY },
      SHIRO_EDGE,
      0.7,
      0.6,
    );
    strokeLine(
      args.ctx,
      { x: chevronX, y: chevronY },
      { x: chevronX + 1.1 * k, y: chevronY + 0.9 * k },
      SHIRO_EDGE,
      0.7,
      0.6,
    );
  }

  if (sleeping && !args.reducedMotion) {
    const zPhase = (frame / 3) % 26;
    if (zPhase < 18) {
      fillPixelRect(
        args.ctx,
        x + bodyW * 0.9,
        baseY - bodyH - zPhase * 0.5,
        2,
        2,
        "#CBD5E1",
        Math.max(0, 0.7 - zPhase * 0.035),
      );
    }
  }
}

function drawSoot(
  args: DrawBuddyWorldBaseArgs,
  companion: BuddyWorldCompanion,
  nowMs: number,
): void {
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const frame = safeFrame(args.frame);
  const position = companionPositionAt(companion, nowMs);
  const jitterX = wave(
    frame,
    5 + (companion.seed % 5),
    companion.seed,
    0.9,
    args.reducedMotion,
  );
  const jitterY = wave(
    frame,
    7 + (companion.seed % 3),
    companion.seed + 4,
    0.7,
    args.reducedMotion,
  );
  const x = pctX(width, position.x) + jitterX;
  const y = pctY(height, position.y) + jitterY;
  const k =
    Math.max(0.4, finiteOr(companion.scale, 1)) * (args.compact ? 0.85 : 1);
  const radius = 3.4 * k * (companion.pose === "peek" ? 0.8 : 1);
  const fleeing = companion.pose === "flee";

  for (let spike = 0; spike < 6; spike += 1) {
    const angle =
      (spike / 6) * Math.PI * 2 +
      seededUnit(companion.seed, spike) * 0.8 +
      (args.reducedMotion ? 0 : frame / 40);
    strokeLine(
      args.ctx,
      { x, y },
      {
        x: x + Math.cos(angle) * radius * 1.7,
        y: y + Math.sin(angle) * radius * 1.7,
      },
      SOOT_BODY,
      0.8,
      0.7,
    );
  }
  fillCircle(args.ctx, x, y, radius, SOOT_BODY, 0.94);
  const eyeOffset = fleeing ? companion.facing * 1.1 : 0;
  fillCircle(
    args.ctx,
    x - 1.2 * k + eyeOffset,
    y - 0.4,
    0.9 * k,
    SOOT_EYE,
    0.95,
  );
  fillCircle(
    args.ctx,
    x + 1.2 * k + eyeOffset,
    y - 0.4,
    0.9 * k,
    SOOT_EYE,
    0.95,
  );
  fillCircle(
    args.ctx,
    x - 1.2 * k + eyeOffset,
    y - 0.4,
    0.4 * k,
    SOOT_BODY,
    0.95,
  );
  fillCircle(
    args.ctx,
    x + 1.2 * k + eyeOffset,
    y - 0.4,
    0.4 * k,
    SOOT_BODY,
    0.95,
  );
  if (fleeing) {
    strokeLine(
      args.ctx,
      { x: x - companion.facing * radius * 2.4, y },
      { x: x - companion.facing * radius * 1.4, y },
      SOOT_BODY,
      0.7,
      0.4,
    );
  }
}

function drawKuro(
  args: DrawBuddyWorldBaseArgs,
  companion: BuddyWorldCompanion,
  nowMs: number,
): void {
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const frame = safeFrame(args.frame);
  const position = companionPositionAt(companion, nowMs);
  const x = pctX(width, position.x);
  const y = pctY(height, position.y);
  const k = args.compact ? 0.85 : 1;
  const fleeing = companion.pose === "flee";
  const flap = fleeing
    ? wave(frame, 2.4, companion.seed, 3.2, args.reducedMotion)
    : wave(frame, 30, companion.seed, 0.6, args.reducedMotion);
  const facing = companion.facing;

  fillEllipse(args.ctx, x, y, 5.4 * k, 3.4 * k, KURO_BODY, 0.96);
  fillCircle(
    args.ctx,
    x + facing * 4.2 * k,
    y - 2.2 * k,
    2.4 * k,
    KURO_BODY,
    0.96,
  );
  fillPixelRect(
    args.ctx,
    x + facing * 6.4 * k - (facing < 0 ? 2.2 * k : 0),
    y - 2.6 * k,
    2.2 * k,
    1.1 * k,
    KURO_BEAK,
    0.92,
  );
  fillCircle(
    args.ctx,
    x + facing * 4.6 * k,
    y - 2.7 * k,
    0.5 * k,
    "#F8FAFC",
    0.9,
  );
  fillEllipse(
    args.ctx,
    x - facing * 1 * k,
    y - 1 * k - flap,
    3.6 * k,
    1.8 * k,
    "#0F172A",
    0.92,
  );
  strokeLine(
    args.ctx,
    { x: x - facing * 4.6 * k, y: y + 0.4 * k },
    { x: x - facing * 7 * k, y: y - 0.6 * k + flap * 0.4 },
    KURO_BODY,
    1.4,
    0.9,
  );
  if (!fleeing) {
    strokeLine(
      args.ctx,
      { x: x - 2 * k, y: y + 3.4 * k },
      { x: x - 2 * k, y: y + 5 * k },
      KURO_BODY,
      0.9,
      0.85,
    );
    strokeLine(
      args.ctx,
      { x: x + 2 * k, y: y + 3.4 * k },
      { x: x + 2 * k, y: y + 5 * k },
      KURO_BODY,
      0.9,
      0.85,
    );
  }
  if (fleeing) {
    fillCircle(
      args.ctx,
      x - facing * 8 * k,
      y + 2 * k,
      1.1 * k,
      "#92400E",
      0.9,
    );
  }
}

export function drawBuddyWorldCompanions(
  args: DrawBuddyWorldBaseArgs,
  companions: readonly BuddyWorldCompanion[],
  nowMs: number,
): void {
  const safeNowMs = finiteOr(nowMs, 0);
  for (const companion of companions) {
    switch (companion.kind) {
      case "shiro":
        drawShiro(args, companion, safeNowMs);
        break;
      case "soot":
        drawSoot(args, companion, safeNowMs);
        break;
      case "kuro":
        drawKuro(args, companion, safeNowMs);
        break;
    }
  }
}

export function drawHomeWindowGlow(args: DrawBuddyWorldBaseArgs): void {
  if (args.world.phase !== "evening" && args.world.phase !== "night") return;
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const frame = safeFrame(args.frame);
  const scale = args.compact ? 0.86 : 1;
  const homeX = pctX(width, BUDDY_WORLD_HOME_HOTSPOT.x);
  const homeY = pctY(height, BUDDY_WORLD_HOME_HOTSPOT.y);
  const windowX = homeX + 9 * scale;
  const windowY = homeY + 5 * scale;
  const windowW = 9 * scale;
  const windowH = 8 * scale;
  const flicker = wave(frame, 24, 1, 0.06, args.reducedMotion);

  fillPixelRect(
    args.ctx,
    windowX,
    windowY,
    windowW,
    windowH,
    "#FDE68A",
    0.78 + flicker,
  );
  fillPixelRect(
    args.ctx,
    windowX + windowW / 2 - 0.5,
    windowY,
    1,
    windowH,
    "#B45309",
    0.5,
  );
  fillPixelRect(
    args.ctx,
    windowX,
    windowY + windowH / 2 - 0.5,
    windowW,
    1,
    "#B45309",
    0.5,
  );
  fillCircle(
    args.ctx,
    windowX + windowW / 2,
    windowY + windowH / 2,
    windowW * 1.4,
    "#FBBF24",
    0.1 + flicker,
  );

  if (args.reducedMotion) return;
  const cyclePosition = (frame + 540) % 1_400;
  if (cyclePosition < 60) {
    const crossProgress = cyclePosition / 60;
    fillCircle(
      args.ctx,
      windowX + windowW * crossProgress,
      windowY + windowH * 0.62,
      1.7 * scale,
      "#0F172A",
      0.85,
    );
  }
}
