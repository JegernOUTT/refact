import {
  fillCircle,
  fillEllipse,
  fillPixel,
  strokeEllipse,
  strokeSeg,
} from "./helpers";
import { drawEyes, drawMouth, drawBrows } from "./eyes";
import type { BuddyAnimState, ColorMap } from "../types";

interface FaceLayout {
  eyeLX: number;
  eyeRX: number;
  eyeY: number;
  eyeSize: number;
  noseX: number;
  noseY: number;
  noseW: number;
  mouthX: number;
  mouthY: number;
  mouthW: number;
  cheekLX: number;
  cheekRX: number;
  cheekY: number;
}

interface TotoroSpec {
  w: number;
  h: number;
  earSpread: number;
  earH: number;
  earW: number;
  chevronRows: Array<[number, number]>;
  face: FaceLayout;
  whiskerY: number;
  armY: number;
}

function totoroRadius(w: number, h: number, r: number): number {
  const t = r / (h - 1);
  const swell = Math.sin(Math.PI * (0.1 + 0.46 * t));
  let radius = (w / 2) * (0.36 + 0.64 * Math.pow(swell, 0.75));
  if (r === 0) radius *= 0.74;
  else if (r === 1) radius *= 0.92;
  if (t > 0.93) radius *= 1 - (t - 0.93) * 1.1;
  return radius;
}

function totoroSmoothRadius(w: number, h: number, r: number): number {
  const clamped = Math.max(0, Math.min(h - 1, r));
  const lower = Math.floor(clamped);
  const upper = Math.min(h - 1, lower + 1);
  const mix = clamped - lower;
  const a = totoroRadius(w, h, lower);
  const b = totoroRadius(w, h, upper);
  return a + (b - a) * mix;
}

function totoroBellySpan(w: number, h: number, r: number): number {
  const t = Math.max(0, Math.min(1, r / (h - 1)));
  const bt = (t - 0.3) / 0.66;
  if (bt < 0 || bt > 1) return -1;
  const radius = totoroSmoothRadius(w, h, r);
  return radius * (0.5 + 0.42 * Math.sin(Math.PI * Math.pow(bt, 0.9)));
}

function traceTotoroBody(
  ctx: CanvasRenderingContext2D,
  ox: number,
  oy: number,
  w: number,
  h: number,
): void {
  const cx = ox + (w - 1) / 2;
  const bottom = h - 1;
  ctx.beginPath();
  ctx.moveTo(cx - totoroSmoothRadius(w, h, 0), oy + 0.3);
  for (let r = 0.5; r <= bottom; r += 0.5) {
    ctx.lineTo(cx - totoroSmoothRadius(w, h, r), oy + r);
  }
  ctx.lineTo(cx - totoroSmoothRadius(w, h, bottom) * 0.8, oy + bottom + 0.6);
  ctx.lineTo(cx + totoroSmoothRadius(w, h, bottom) * 0.8, oy + bottom + 0.6);
  for (let r = bottom; r >= 0.5; r -= 0.5) {
    ctx.lineTo(cx + totoroSmoothRadius(w, h, r), oy + r);
  }
  ctx.lineTo(cx + totoroSmoothRadius(w, h, 0), oy + 0.3);
}

function traceTotoroBelly(
  ctx: CanvasRenderingContext2D,
  ox: number,
  oy: number,
  w: number,
  h: number,
): void {
  const cx = ox + (w - 1) / 2;
  const samples: Array<[number, number]> = [];
  for (let r = 0; r <= h - 1; r += 0.5) {
    const span = totoroBellySpan(w, h, r);
    if (span > 0.3) samples.push([r, span]);
  }
  if (samples.length < 2) return;
  ctx.beginPath();
  ctx.moveTo(cx - samples[0][1], oy + samples[0][0]);
  for (const [r, span] of samples) {
    ctx.lineTo(cx - span, oy + r);
  }
  for (let index = samples.length - 1; index >= 0; index -= 1) {
    const [r, span] = samples[index];
    ctx.lineTo(cx + span, oy + r);
  }
}

function drawTotoroBodySmooth(
  ctx: CanvasRenderingContext2D,
  ox: number,
  oy: number,
  w: number,
  h: number,
  m: ColorMap,
): void {
  ctx.save();
  traceTotoroBody(ctx, ox, oy, w, h);
  ctx.fillStyle = m.dark;
  ctx.fill();
  traceTotoroBody(ctx, ox, oy, w, h);
  ctx.clip();
  traceTotoroBody(ctx, ox - 0.7, oy - 1.1, w, h);
  ctx.fillStyle = m.body;
  ctx.fill();
  traceTotoroBody(ctx, ox - 1.7, oy - 2.5, w, h);
  ctx.fillStyle = m.light;
  ctx.fill();
  traceTotoroBelly(ctx, ox, oy, w, h);
  ctx.fillStyle = m.light;
  ctx.fill();
  traceTotoroBelly(ctx, ox, oy - 1.3, w, h);
  ctx.fillStyle = m.belly;
  ctx.fill();
  ctx.restore();
  ctx.save();
  traceTotoroBody(ctx, ox, oy, w, h);
  ctx.strokeStyle = m.outline;
  ctx.lineWidth = 0.85;
  ctx.lineJoin = "round";
  ctx.stroke();
  ctx.restore();
}

function faceOffset(anim: BuddyAnimState): number {
  return anim.facingLerp * 1.6;
}

function drawFace(
  ctx: CanvasRenderingContext2D,
  ox: number,
  oy: number,
  m: ColorMap,
  anim: BuddyAnimState,
  layout: FaceLayout,
): void {
  const off = faceOffset(anim);
  drawEyes(
    ctx,
    ox + layout.eyeLX + off,
    oy + layout.eyeY,
    ox + layout.eyeRX + off,
    oy + layout.eyeY,
    m,
    layout.eyeSize,
    anim,
  );
  drawBrows(
    ctx,
    ox + layout.eyeLX + off,
    oy + layout.eyeY,
    ox + layout.eyeRX + off,
    oy + layout.eyeY,
    layout.eyeSize,
    m,
    anim,
  );
  const noseCx = ox + layout.noseX + layout.noseW / 2 + off;
  const noseTopY = oy + layout.noseY - 0.2;
  ctx.beginPath();
  ctx.moveTo(noseCx - layout.noseW * 0.8, noseTopY);
  ctx.lineTo(noseCx + layout.noseW * 0.8, noseTopY);
  ctx.lineTo(noseCx + layout.noseW * 0.24, noseTopY + 1.8);
  ctx.lineTo(noseCx - layout.noseW * 0.24, noseTopY + 1.8);
  ctx.fillStyle = m.eyeDark;
  ctx.fill();
  ctx.globalAlpha = 0.55;
  fillEllipse(ctx, noseCx - 0.5, noseTopY + 0.5, 0.5, 0.3, m.light);
  ctx.globalAlpha = 1;
  fillEllipse(
    ctx,
    ox + layout.cheekLX + 1 + off,
    oy + layout.cheekY + 0.5,
    1.4,
    0.7,
    m.rosy,
  );
  fillEllipse(
    ctx,
    ox + layout.cheekRX + 1 + off,
    oy + layout.cheekY + 0.5,
    1.4,
    0.7,
    m.rosy,
  );
  drawMouth(
    ctx,
    ox + layout.mouthX + off,
    oy + layout.mouthY,
    m,
    layout.mouthW,
    anim,
  );
}

function drawTotoroEars(
  ctx: CanvasRenderingContext2D,
  cx: number,
  topY: number,
  spread: number,
  earH: number,
  earW: number,
  m: ColorMap,
  anim: BuddyAnimState,
): void {
  const lift = Math.max(0, anim.earAnimProgress) * 2;
  const droop = anim.earAnimProgress < -0.3 ? 1.4 : 0;
  const twitch = anim.earTwitchTimer > 0 && anim.earTwitchTimer % 4 < 2 ? 1 : 0;
  const off = faceOffset(anim);
  const baseHalf = earW / 2 + 1.7;
  const height = earH * 2.35;

  const ear = (
    ecx: number,
    tw: number,
    tilt: number,
    lighter: boolean,
  ): void => {
    const baseY = topY + 2.4 + droop - lift - tw;
    const tipX = ecx + tilt * (1.2 + height * 0.14);
    const tipY = baseY - height;
    const midY = baseY - height * 0.54;
    const midL = ecx - baseHalf * 0.72 + tilt * height * 0.06;
    const midR = ecx + baseHalf * 0.72 + tilt * height * 0.1;

    ctx.beginPath();
    ctx.moveTo(ecx - baseHalf, baseY);
    ctx.lineTo(midL, midY);
    ctx.lineTo(tipX - 0.55, tipY + 0.8);
    ctx.lineTo(tipX, tipY);
    ctx.lineTo(tipX + 0.55, tipY + 0.8);
    ctx.lineTo(midR, midY);
    ctx.lineTo(ecx + baseHalf, baseY);
    ctx.fillStyle = lighter ? m.light : m.body;
    ctx.fill();
    ctx.strokeStyle = m.outline;
    ctx.lineWidth = 0.8;
    ctx.lineJoin = "round";
    ctx.stroke();
    ctx.globalAlpha = 0.34;
    fillEllipse(
      ctx,
      ecx + tilt * 0.7,
      baseY - height * 0.4,
      baseHalf * 0.4,
      height * 0.3,
      m.dark,
      tilt * 0.16,
    );
    ctx.globalAlpha = 1;
  };

  ear(cx - spread + off, anim.earTwitchSide < 0 ? twitch : 0, -1, true);
  ear(cx + spread + off, anim.earTwitchSide > 0 ? twitch : 0, 1, false);
}

function drawTotoroTail(
  ctx: CanvasRenderingContext2D,
  bodyX: number,
  bodyW: number,
  anchorY: number,
  m: ColorMap,
  anim: BuddyAnimState,
): void {
  if (anim.idleAction === "doze") {
    fillEllipse(ctx, bodyX - 1, anchorY + 4, 2.6, 1.5, m.outline);
    fillEllipse(ctx, bodyX - 1, anchorY + 3.8, 2.2, 1.2, m.body);
    fillEllipse(ctx, bodyX - 1.8, anchorY + 4.4, 1, 0.5, m.dark);
    return;
  }
  const facingRight = anim.facingLerp >= 0;
  const x = facingRight ? bodyX - 4 : bodyX + bodyW - 1;
  const bob = Math.sin(anim.tailPhase) * (0.4 + anim.tailEnergy * 0.9);
  const sag = anim.tailDroop * 2;
  const y = anchorY + sag - bob;
  fillCircle(ctx, x + 2.5, y + 2, 2.6, m.outline);
  fillCircle(ctx, x + 2.5, y + 2, 2.1, m.body);
  fillEllipse(ctx, x + 1.7, y + 1.2, 1, 0.8, m.light);
  fillEllipse(ctx, x + 3.2, y + 3.2, 1, 0.6, m.dark);
}

function drawLegs(
  ctx: CanvasRenderingContext2D,
  cx: number,
  footY: number,
  m: ColorMap,
  anim: BuddyAnimState,
): void {
  const foot = (x: number, y: number): void => {
    fillEllipse(ctx, x + 2.5, y + 0.2, 3, 1.9, m.outline);
    fillEllipse(ctx, x + 2.5, y, 2.7, 1.6, m.body);
    fillEllipse(ctx, x + 1.6, y - 0.5, 1.2, 0.7, m.light);
    fillCircle(ctx, x + 1.5, y + 1, 0.34, m.outline);
    fillCircle(ctx, x + 3.5, y + 1, 0.34, m.outline);
  };
  if (anim.idleAction === "doze") {
    fillEllipse(ctx, cx - 4.5, footY + 0.4, 2.7, 0.9, m.dark);
    fillEllipse(ctx, cx + 4.5, footY + 0.4, 2.7, 0.9, m.dark);
    return;
  }
  if (anim.walking && Math.abs(anim.walkVel) > 0.08) {
    const liftA = Math.max(0, Math.sin(anim.walkPhase)) * 2.6;
    const liftB = Math.max(0, Math.sin(anim.walkPhase + Math.PI)) * 2.6;
    const strideA = Math.cos(anim.walkPhase) * 1.8 * anim.walkDirection;
    const strideB =
      Math.cos(anim.walkPhase + Math.PI) * 1.8 * anim.walkDirection;
    foot(cx - 7 + strideA, footY - liftA);
    foot(cx + 2 + strideB, footY - liftB);
    return;
  }
  if (anim.idleAction === "dance") {
    const hop = Math.sin(anim.dancePhase);
    foot(cx - 7, footY - Math.max(0, hop) * 3);
    foot(cx + 2, footY - Math.max(0, -hop) * 3);
    return;
  }
  const shift = Math.sin(anim.frame * 0.013);
  foot(cx - 7, footY - (shift > 0.6 ? 1 : 0));
  foot(cx + 2, footY - (shift < -0.6 ? 1 : 0));
}

function drawArms(
  ctx: CanvasRenderingContext2D,
  lx: number,
  rx: number,
  midY: number,
  m: ColorMap,
  anim: BuddyAnimState,
): void {
  const f = anim.frame;
  const dir = anim.facingLerp >= 0 ? 1 : -1;
  const hand = (x: number, y: number): void => {
    fillCircle(ctx, x + 1, y + 0.7, 1.4, m.outline);
    fillCircle(ctx, x + 1, y + 0.7, 1.05, m.belly);
  };
  const limb = (
    x1: number,
    y1: number,
    x2: number,
    y2: number,
    width = 2.3,
  ): void => {
    strokeSeg(ctx, x1, y1, x2, y2, m.outline, width + 0.8);
    strokeSeg(ctx, x1, y1, x2, y2, m.body, width);
  };
  const restArm = (x: number, y: number): void => {
    limb(x + 1, y + 0.6, x + 1.1, y + 3.2, 2.5);
    fillCircle(ctx, x + 1.1, y + 3.5, 1.05, m.body);
  };

  switch (anim.armPose) {
    case "swing": {
      const sw = Math.sin(anim.walkPhase) * 2;
      restArm(lx, midY - sw);
      restArm(rx, midY + sw);
      return;
    }
    case "wave": {
      const wx = dir > 0 ? rx : lx;
      const sx = dir > 0 ? lx : rx;
      const bob = Math.sin(f * 0.28) * 1.5;
      restArm(sx, midY);
      limb(wx + dir, midY + 0.6, wx + dir * 2.6, midY - 3);
      limb(wx + dir * 2.6, midY - 3, wx + dir * 3.6, midY - 5.6 - bob);
      hand(wx + dir * 3.2, midY - 7.2 - bob);
      return;
    }
    case "raise": {
      const bounce = Math.abs(Math.sin(f * 0.18)) * 2;
      limb(lx + 1, midY - 1, lx - 1.4, midY - 5 - bounce);
      hand(lx - 2.2, midY - 6.8 - bounce);
      limb(rx + 1, midY - 1, rx + 3.4, midY - 5 - bounce);
      hand(rx + 2.2, midY - 6.8 - bounce);
      return;
    }
    case "face": {
      const wx = dir > 0 ? rx : lx;
      const sx = dir > 0 ? lx : rx;
      restArm(sx, midY);
      limb(wx + 1, midY + 0.6, wx - dir + 1, midY - 3.4);
      hand(wx - dir, midY - 5);
      return;
    }
    case "drum": {
      const beat = Math.sin(f * 0.38) > 0 ? 1 : 0;
      limb(lx + 1.4, midY + 2, lx + 4, midY + 4.4 + beat);
      hand(lx + 3.6, midY + 5 + beat);
      limb(rx + 0.6, midY + 2, rx - 2, midY + 5.4 - beat);
      hand(rx - 2.6, midY + 6 - beat);
      return;
    }
    case "hold": {
      const wx = dir > 0 ? rx : lx;
      const sx = dir > 0 ? lx : rx;
      restArm(sx, midY);
      limb(wx + 1, midY + 1.4, wx + dir * 4 + 1, midY + 1.8);
      hand(wx + dir * 4, midY + 1);
      return;
    }
    case "hug": {
      limb(lx + 1, midY + 1.4, lx + 4.4, midY + 3.6);
      limb(rx + 1, midY + 1.4, rx - 2.4, midY + 3.6);
      hand(lx + 4, midY + 3.6);
      hand(rx - 3, midY + 3.6);
      return;
    }
    default: {
      const breath = anim.breathScale * 90;
      restArm(lx, midY + breath);
      restArm(rx, midY + breath);
    }
  }
}

function drawFurTufts(
  ctx: CanvasRenderingContext2D,
  ox: number,
  oy: number,
  w: number,
  h: number,
  m: ColorMap,
  anim: BuddyAnimState,
): void {
  const cx = ox + (w - 1) / 2;
  const ruffle = 0.05 + anim.wingFlap * 0.12;
  const rows = [0.3, 0.46, 0.62, 0.78];
  for (let i = 0; i < rows.length; i++) {
    const r = rows[i] * (h - 1);
    const ex = totoroSmoothRadius(w, h, r);
    const flick = Math.sin(anim.frame * ruffle + i * 1.9) > 0.55 ? 0.7 : 0;
    fillEllipse(ctx, cx - ex - 0.5 - flick, oy + r, 0.85, 0.5, m.body);
    fillEllipse(ctx, cx + ex + 0.5 + flick, oy + r, 0.85, 0.5, m.body);
    if (i % 2 === 0) {
      fillEllipse(ctx, cx - ex - 0.4, oy + r + 1, 0.6, 0.4, m.dark);
      fillEllipse(ctx, cx + ex + 0.4, oy + r + 1, 0.6, 0.4, m.dark);
    }
  }
}

function drawChevronRow(
  ctx: CanvasRenderingContext2D,
  cx: number,
  y: number,
  m: ColorMap,
  count: number,
): void {
  const gap = 4.8;
  const span = (count - 1) * gap;
  for (let i = 0; i < count; i++) {
    const x = cx - span / 2 + i * gap;
    ctx.beginPath();
    ctx.moveTo(x - 1.95, y + 2.5);
    ctx.lineTo(x - 0.2, y + 0.1);
    ctx.lineTo(x + 0.2, y + 0.1);
    ctx.lineTo(x + 1.95, y + 2.5);
    ctx.lineTo(x + 1.05, y + 2.6);
    ctx.lineTo(x, y + 1.25);
    ctx.lineTo(x - 1.05, y + 2.6);
    ctx.fillStyle = m.body;
    ctx.fill();
  }
}

function drawTotoroWhiskers(
  ctx: CanvasRenderingContext2D,
  lx: number,
  rx: number,
  y: number,
  m: ColorMap,
  anim: BuddyAnimState,
): void {
  const t = anim.earTwitchTimer > 0 && anim.earTwitchTimer % 4 < 2 ? 0.8 : 0;
  const off = faceOffset(anim);
  const L = lx + off;
  const R = rx + off;
  strokeSeg(ctx, L - 7.2, y - 2.6 - t, L - 0.4, y - 1.4 - t, m.outline, 0.5);
  strokeSeg(ctx, L - 8, y + 0.4, L - 0.4, y, m.outline, 0.5);
  strokeSeg(ctx, L - 7.2, y + 3.2, L - 0.4, y + 1.6, m.outline, 0.5);
  strokeSeg(ctx, R + 0.4, y - 1.4 - t, R + 7.2, y - 2.6 - t, m.outline, 0.5);
  strokeSeg(ctx, R + 0.4, y, R + 8, y + 0.4, m.outline, 0.5);
  strokeSeg(ctx, R + 0.4, y + 1.6, R + 7.2, y + 3.2, m.outline, 0.5);
}

function drawLeafHat(
  ctx: CanvasRenderingContext2D,
  cx: number,
  topY: number,
  anim: BuddyAnimState,
): void {
  const sway = Math.sin(anim.frame * 0.05) * 0.5;
  fillEllipse(ctx, cx + 0.4 + sway, topY - 0.7, 4.4, 1.6, "#5C9450", -0.13);
  fillEllipse(ctx, cx - 0.6 + sway, topY - 1.1, 2.3, 0.85, "#79B26A", -0.13);
  strokeSeg(
    ctx,
    cx - 3.6 + sway,
    topY - 0.3,
    cx + 4 + sway,
    topY - 1.3,
    "#3F6B35",
    0.45,
  );
  strokeSeg(
    ctx,
    cx + 3.9 + sway,
    topY - 1.25,
    cx + 5.3 + sway,
    topY - 2,
    "#3F6B35",
    0.6,
  );
}

function drawLeafUmbrella(
  ctx: CanvasRenderingContext2D,
  cx: number,
  topY: number,
  anim: BuddyAnimState,
): void {
  const sway = Math.sin(anim.frame * 0.04) * 1.5;
  strokeSeg(ctx, cx, topY + 1.4, cx, topY - 3.4, "#3F6B35", 0.9);
  fillEllipse(ctx, cx + sway, topY - 4.4, 7, 2.3, "#4A7D40", -0.08);
  fillEllipse(ctx, cx + sway, topY - 4.8, 6.4, 1.8, "#5C9450", -0.08);
  fillEllipse(ctx, cx - 2 + sway, topY - 5.2, 2.8, 0.85, "#79B26A", -0.12);
  strokeSeg(
    ctx,
    cx - 6 + sway,
    topY - 3.8,
    cx + 6 + sway,
    topY - 5.4,
    "#3F6B35",
    0.5,
  );
}

function whiskerEdges(
  spec: TotoroSpec,
  ox: number,
): { lx: number; rx: number } {
  const cx = ox + (spec.w - 1) / 2;
  const radius = totoroRadius(spec.w, spec.h, spec.whiskerY);
  return {
    lx: cx - radius + 1,
    rx: cx + radius,
  };
}

function armEdges(spec: TotoroSpec, ox: number): { lx: number; rx: number } {
  const cx = ox + (spec.w - 1) / 2;
  const radius = totoroRadius(spec.w, spec.h, spec.armY);
  return {
    lx: cx - radius * 0.92,
    rx: cx + radius * 0.92 - 1,
  };
}

function drawTotoro(
  ctx: CanvasRenderingContext2D,
  ox: number,
  oy: number,
  m: ColorMap,
  anim: BuddyAnimState,
  spec: TotoroSpec,
): void {
  const cx = ox + Math.round(spec.w / 2);
  drawTotoroTail(
    ctx,
    ox + 1,
    spec.w - 2,
    oy + Math.round(spec.h * 0.6),
    m,
    anim,
  );
  drawTotoroEars(
    ctx,
    cx - 1,
    oy + 2,
    spec.earSpread,
    spec.earH,
    spec.earW,
    m,
    anim,
  );
  drawTotoroBodySmooth(ctx, ox, oy, spec.w, spec.h, m);
  drawFurTufts(ctx, ox, oy, spec.w, spec.h, m, anim);
  for (const [count, y] of spec.chevronRows) {
    drawChevronRow(ctx, cx - 1, oy + y, m, count);
  }
  const { lx, rx } = whiskerEdges(spec, ox);
  drawTotoroWhiskers(ctx, lx, rx, oy + spec.whiskerY, m, anim);
  drawLegs(ctx, cx - 1, oy + spec.h - 1, m, anim);
  drawFace(ctx, ox, oy, m, anim, spec.face);
  const arms = armEdges(spec, ox);
  drawArms(ctx, arms.lx, arms.rx, oy + spec.armY, m, anim);
}

const SPEC_SPRITE: TotoroSpec = {
  w: 22,
  h: 19,
  earSpread: 4,
  earH: 5,
  earW: 2,
  chevronRows: [
    [3, 9],
    [2, 12],
  ],
  face: {
    eyeLX: 4,
    eyeRX: 15,
    eyeY: 3,
    eyeSize: 3,
    noseX: 10,
    noseY: 5,
    noseW: 2,
    mouthX: 9,
    mouthY: 7,
    mouthW: 4,
    cheekLX: 3,
    cheekRX: 17,
    cheekY: 6,
  },
  whiskerY: 5,
  armY: 9,
};

const SPEC_IMP: TotoroSpec = {
  w: 24,
  h: 20,
  earSpread: 4,
  earH: 6,
  earW: 2,
  chevronRows: [
    [3, 10],
    [2, 13],
  ],
  face: {
    eyeLX: 5,
    eyeRX: 16,
    eyeY: 3,
    eyeSize: 3,
    noseX: 11,
    noseY: 5,
    noseW: 2,
    mouthX: 10,
    mouthY: 8,
    mouthW: 4,
    cheekLX: 3,
    cheekRX: 19,
    cheekY: 7,
  },
  whiskerY: 6,
  armY: 9,
};

const SPEC_DAEMON: TotoroSpec = {
  w: 26,
  h: 21,
  earSpread: 5,
  earH: 7,
  earW: 2,
  chevronRows: [
    [4, 10],
    [3, 13],
  ],
  face: {
    eyeLX: 5,
    eyeRX: 18,
    eyeY: 3,
    eyeSize: 3,
    noseX: 11,
    noseY: 5,
    noseW: 3,
    mouthX: 11,
    mouthY: 8,
    mouthW: 4,
    cheekLX: 3,
    cheekRX: 21,
    cheekY: 7,
  },
  whiskerY: 6,
  armY: 10,
};

const SPEC_ARCHON: TotoroSpec = {
  w: 28,
  h: 23,
  earSpread: 5,
  earH: 7,
  earW: 2,
  chevronRows: [
    [4, 11],
    [3, 14],
  ],
  face: {
    eyeLX: 6,
    eyeRX: 19,
    eyeY: 4,
    eyeSize: 3,
    noseX: 12,
    noseY: 6,
    noseW: 3,
    mouthX: 12,
    mouthY: 9,
    mouthW: 4,
    cheekLX: 4,
    cheekRX: 22,
    cheekY: 8,
  },
  whiskerY: 7,
  armY: 11,
};

const EGG_W = 20;
const EGG_H = 16;

const EGG_SPECKLES = [
  [6, 8],
  [13, 7],
  [9, 10],
  [5, 9],
  [14, 12],
  [8, 13],
] as const;

function eggRadius(w: number, h: number, r: number): number {
  const t = Math.max(0, Math.min(1, r / (h - 1)));
  return (w / 2) * (0.52 + 0.48 * Math.sin(Math.PI * (0.18 + t * 0.66)));
}

function traceEgg(
  ctx: CanvasRenderingContext2D,
  ox: number,
  oy: number,
  w: number,
  h: number,
): void {
  const cx = ox + (w - 1) / 2;
  const bottom = h - 1;
  ctx.beginPath();
  ctx.moveTo(cx - eggRadius(w, h, 0), oy + 0.3);
  for (let r = 0.5; r <= bottom; r += 0.5) {
    ctx.lineTo(cx - eggRadius(w, h, r), oy + r);
  }
  ctx.lineTo(cx - eggRadius(w, h, bottom) * 0.7, oy + bottom + 0.5);
  ctx.lineTo(cx + eggRadius(w, h, bottom) * 0.7, oy + bottom + 0.5);
  for (let r = bottom; r >= 0.5; r -= 0.5) {
    ctx.lineTo(cx + eggRadius(w, h, r), oy + r);
  }
  ctx.lineTo(cx + eggRadius(w, h, 0), oy + 0.3);
}

export function drawEgg(
  ctx: CanvasRenderingContext2D,
  ox: number,
  oy: number,
  m: ColorMap,
  anim: BuddyAnimState,
  _paletteIndex: number,
): void {
  const crack = Math.min(anim.frame / 30 / 10, 1);
  const rock = Math.sin(anim.frame * 0.04) * 1.5;
  const x = ox + rock;

  ctx.save();
  traceEgg(ctx, x, oy, EGG_W, EGG_H);
  ctx.fillStyle = m.dark;
  ctx.fill();
  traceEgg(ctx, x, oy, EGG_W, EGG_H);
  ctx.clip();
  traceEgg(ctx, x - 0.7, oy - 1, EGG_W, EGG_H);
  ctx.fillStyle = m.body;
  ctx.fill();
  traceEgg(ctx, x - 1.6, oy - 2.2, EGG_W, EGG_H);
  ctx.fillStyle = m.light;
  ctx.fill();

  ctx.beginPath();
  ctx.moveTo(x - 1, oy - 2);
  for (let c = 0; c <= EGG_W; c += 2) {
    ctx.lineTo(x + c, oy + (c % 4 === 0 ? 5.2 : 4));
  }
  ctx.lineTo(x + EGG_W + 1, oy - 2);
  ctx.fillStyle = m.dark;
  ctx.fill();
  for (let c = 0; c <= EGG_W - 2; c += 2) {
    strokeSeg(
      ctx,
      x + c,
      oy + (c % 4 === 0 ? 5.2 : 4),
      x + c + 2,
      oy + ((c + 2) % 4 === 0 ? 5.2 : 4),
      m.outline,
      0.55,
    );
  }
  ctx.restore();

  ctx.save();
  traceEgg(ctx, x, oy, EGG_W, EGG_H);
  ctx.strokeStyle = m.outline;
  ctx.lineWidth = 0.8;
  ctx.lineJoin = "round";
  ctx.stroke();
  ctx.restore();

  for (const [sx, sy] of EGG_SPECKLES) {
    fillEllipse(ctx, x + sx + 0.5, oy + sy + 0.5, 0.7, 0.5, m.light);
  }
  fillEllipse(ctx, x + 10, oy - 2, 1.4, 1.2, m.outline);
  fillEllipse(ctx, x + 10.2, oy - 2.9, 1.6, 0.7, m.dark);
  fillEllipse(ctx, x + 6, oy + 1.4, 1.2, 0.55, m.light);

  if (crack > 0.1) {
    const d = Math.floor(crack * 8);
    const cx = x + 10;
    const segments: Array<[number, number, number, number]> = [
      [cx, oy + 6, cx - 1, oy + 7.2],
      [cx - 1, oy + 7.2, cx, oy + 8.4],
      [cx, oy + 8.4, cx + 1, oy + 9.6],
      [cx + 1, oy + 9.6, cx, oy + 10.6],
      [cx, oy + 10.6, cx - 1, oy + 11.8],
      [cx - 1, oy + 11.8, cx, oy + 12.8],
      [cx, oy + 12.8, cx + 1, oy + 13.8],
    ];
    for (let index = 0; index < Math.min(d, segments.length); index += 1) {
      const [x1, y1, x2, y2] = segments[index];
      strokeSeg(ctx, x1, y1, x2, y2, m.outline, 0.6);
    }
  }
  if (crack > 0.5) {
    ctx.globalAlpha = Math.min(1, (crack - 0.5) * 3);
    fillEllipse(ctx, x + 8, oy + 10, 1.1, 1.1, m.eyeDark);
    fillEllipse(ctx, x + 14, oy + 10, 1.1, 1.1, m.eyeDark);
    ctx.globalAlpha = 1;
  }
}

function paleColorMap(m: ColorMap): ColorMap {
  return {
    ...m,
    body: m.belly,
    light: m.white,
    dark: m.light,
    belly: m.white,
  };
}

export function drawHatch(
  ctx: CanvasRenderingContext2D,
  ox: number,
  oy: number,
  m: ColorMap,
  anim: BuddyAnimState,
): void {
  const bodyY = oy + 7;
  const pale = paleColorMap(m);
  const bx = ox + 2;
  const cx = bx + 8;

  drawTotoroEars(ctx, cx - 1, bodyY, 3, 3, 2, pale, anim);
  drawTotoroBodySmooth(ctx, bx, bodyY, 16, 12, pale);

  const off = faceOffset(anim);
  const hatTilt = Math.sin(anim.frame * 0.02);
  const hatX = bx + 7.5 + off + hatTilt;
  ctx.fillStyle = m.belly;
  ctx.beginPath();
  ctx.ellipse(hatX, bodyY - 0.6, 5.4, 3, 0, Math.PI, 0, false);
  ctx.fill();
  for (let i = 0; i < 4; i += 1) {
    fillCircle(ctx, hatX - 4 + i * 2.7, bodyY - 0.4, 1.05, m.belly);
  }
  fillEllipse(ctx, hatX - 2.4, bodyY - 2.2, 1.4, 0.7, m.white);

  drawFace(ctx, bx, bodyY, pale, anim, {
    eyeLX: 3,
    eyeRX: 11,
    eyeY: 3,
    eyeSize: 2,
    noseX: 7,
    noseY: 4,
    noseW: 2,
    mouthX: 6,
    mouthY: 6,
    mouthW: 3,
    cheekLX: 1,
    cheekRX: 13,
    cheekY: 5,
  });

  const shellY = oy + 19.4;
  ctx.fillStyle = m.belly;
  ctx.beginPath();
  ctx.ellipse(ox + 10, shellY, 9.2, 3.4, 0, 0, Math.PI, false);
  ctx.fill();
  for (let i = 0; i < 5; i += 1) {
    fillCircle(ctx, ox + 3 + i * 3.6, shellY + 0.2, 1.15, m.belly);
  }
  fillEllipse(ctx, ox + 10, shellY + 2.2, 8.6, 0.8, m.light);
}

export function drawSprite(
  ctx: CanvasRenderingContext2D,
  ox: number,
  oy: number,
  m: ColorMap,
  anim: BuddyAnimState,
): void {
  if (anim.quirkActive && anim.quirkType === "phase")
    ctx.globalAlpha = anim.phaseAlpha;

  drawTotoro(ctx, ox, oy, m, anim, SPEC_SPRITE);
  drawLeafHat(ctx, ox + 11, oy + 1, anim);

  if (anim.quirkActive && anim.quirkType === "phase") ctx.globalAlpha = 1;
}

export function drawImp(
  ctx: CanvasRenderingContext2D,
  ox: number,
  oy: number,
  m: ColorMap,
  anim: BuddyAnimState,
): void {
  drawTotoro(ctx, ox, oy, m, anim, SPEC_IMP);
  const off = faceOffset(anim);
  if (anim.moodType !== "concerned" && anim.idleAction !== "doze") {
    fillPixel(ctx, ox + 13 + off, oy + 9, 1, 1, m.white);
  }
}

export function drawDaemon(
  ctx: CanvasRenderingContext2D,
  ox: number,
  oy: number,
  m: ColorMap,
  anim: BuddyAnimState,
): void {
  drawTotoro(ctx, ox, oy, m, anim, SPEC_DAEMON);
}

export function drawSage(
  ctx: CanvasRenderingContext2D,
  ox: number,
  oy: number,
  m: ColorMap,
  anim: BuddyAnimState,
): void {
  drawTotoro(ctx, ox, oy, m, anim, SPEC_DAEMON);
  drawLeafUmbrella(ctx, ox + 12, oy, anim);

  const off = faceOffset(anim);
  fillPixel(ctx, ox + 11 + off, oy + 7, 1, 1, m.white);
  fillPixel(ctx, ox + 14 + off, oy + 7, 1, 1, m.white);

  if (anim.auraPulseIntensity > 0) {
    ctx.globalAlpha = anim.auraPulseIntensity * 0.3;
    const r = 13 + Math.sin(anim.frame * 0.05) * 3;
    strokeEllipse(ctx, ox + 13, oy + 10, r, r * 0.78, m.gold);
    ctx.globalAlpha = 1;
  }
}

export function drawArchon(
  ctx: CanvasRenderingContext2D,
  ox: number,
  oy: number,
  m: ColorMap,
  anim: BuddyAnimState,
): void {
  const f = anim.frame;
  drawTotoro(ctx, ox, oy, m, anim, SPEC_ARCHON);

  const cx = ox + 13.5;
  const crownY = oy - 2;
  ctx.fillStyle = m.gold;
  ctx.beginPath();
  ctx.moveTo(cx - 2.4, crownY + 1.6);
  ctx.lineTo(cx - 2.4, crownY - 0.6);
  ctx.lineTo(cx - 1.2, crownY + 0.4);
  ctx.lineTo(cx, crownY - 1.4);
  ctx.lineTo(cx + 1.2, crownY + 0.4);
  ctx.lineTo(cx + 2.4, crownY - 0.6);
  ctx.lineTo(cx + 2.4, crownY + 1.6);
  ctx.fill();
  fillCircle(ctx, cx, crownY - 1.8, 0.5, m.outline);

  const glow = 0.18 + Math.sin(f * 0.07) * 0.1;
  ctx.globalAlpha = glow;
  fillEllipse(ctx, cx - 0.5, oy + 14, 2.8, 1.6, m.gold);
  ctx.globalAlpha = 1;

  for (let i = 0; i < 4; i++) {
    const a = f * 0.02 + i * 1.57;
    ctx.globalAlpha = 0.5 + Math.sin(f * 0.04 + i) * 0.3;
    fillCircle(
      ctx,
      ox + 14 + Math.cos(a) * 18,
      oy + 11 + Math.sin(a) * 10,
      1,
      m.gold,
    );
    ctx.globalAlpha = 1;
  }
}

export function drawStageCharacter(
  ctx: CanvasRenderingContext2D,
  stage: number,
  ox: number,
  oy: number,
  m: ColorMap,
  anim: BuddyAnimState,
  paletteIndex: number,
): void {
  switch (stage) {
    case 0:
      drawEgg(ctx, ox, oy, m, anim, paletteIndex);
      break;
    case 1:
      drawHatch(ctx, ox, oy, m, anim);
      break;
    case 2:
      drawSprite(ctx, ox, oy, m, anim);
      break;
    case 3:
      drawImp(ctx, ox, oy, m, anim);
      break;
    case 4:
      drawDaemon(ctx, ox, oy, m, anim);
      break;
    case 5:
      drawSage(ctx, ox, oy, m, anim);
      break;
    case 6:
      drawArchon(ctx, ox, oy, m, anim);
      break;
  }
}
