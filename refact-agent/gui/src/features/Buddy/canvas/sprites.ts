import { fillPixel, strokeEllipse } from "./helpers";
import { drawEyes, drawMouth, drawBrows } from "./eyes";
import type { BuddyAnimState, ColorMap } from "../types";

type CellCode = "O" | "L" | "B" | "D" | "W" | "S" | "H" | " ";

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
  grid: CellCode[][];
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
  if (t > 0.93) radius *= 1 - (t - 0.93) * 2.4;
  return radius;
}

function totoroMask(w: number, h: number): string[] {
  const rows: string[] = [];
  const cx = (w - 1) / 2;
  for (let r = 0; r < h; r++) {
    const t = r / (h - 1);
    const radius = totoroRadius(w, h, r);
    const bt = (t - 0.3) / 0.66;
    const bellySpan =
      bt >= 0 && bt <= 1
        ? radius * (0.5 + 0.42 * Math.sin(Math.PI * Math.pow(bt, 0.9)))
        : -1;
    let row = "";
    for (let c = 0; c < w; c++) {
      const d = Math.abs(c - cx);
      if (d > radius) {
        row += " ";
        continue;
      }
      row += d <= bellySpan ? "W" : "X";
    }
    rows.push(row);
  }
  return rows;
}

function eggMask(w: number, h: number): string[] {
  const rows: string[] = [];
  const cx = (w - 1) / 2;
  for (let r = 0; r < h; r++) {
    const t = r / (h - 1);
    const radius =
      (w / 2) * (0.52 + 0.48 * Math.sin(Math.PI * (0.18 + t * 0.66)));
    let row = "";
    for (let c = 0; c < w; c++) {
      row += Math.abs(c - cx) <= radius ? "X" : " ";
    }
    rows.push(row);
  }
  return rows;
}

function compileShaded(rows: string[]): CellCode[][] {
  const h = rows.length;
  const w = Math.max(...rows.map((row) => row.length));
  const at = (c: number, r: number): string => {
    if (r < 0 || r >= h || c < 0 || c >= w) return " ";
    return rows[r][c] ?? " ";
  };
  let bellyBottom = -1;
  for (let r = 0; r < h; r++) {
    if (rows[r].includes("W")) bellyBottom = r;
  }
  const grid: CellCode[][] = [];
  for (let r = 0; r < h; r++) {
    const out: CellCode[] = [];
    for (let c = 0; c < w; c++) {
      const ch = at(c, r);
      if (ch === " ") {
        out.push(" ");
        continue;
      }
      const edge =
        at(c - 1, r) === " " ||
        at(c + 1, r) === " " ||
        at(c, r - 1) === " " ||
        at(c, r + 1) === " ";
      if (edge) {
        out.push("O");
        continue;
      }
      if (ch === "W") {
        out.push(r >= bellyBottom - 1 ? "S" : "W");
        continue;
      }
      const nx = (c - (w - 1) / 2) / (w / 2);
      const ny = (r - (h - 1) / 2) / (h / 2);
      const d = -nx * 0.62 - ny * 0.82;
      if (d > 0.66) out.push("H");
      else if (d > 0.22) out.push("L");
      else if (d < -0.36) out.push("D");
      else out.push("B");
    }
    grid.push(out);
  }
  return grid;
}

const GRID_EGG = compileShaded(eggMask(20, 16));
const GRID_HATCHLING = compileShaded(totoroMask(16, 12));
const GRID_SPRITE = compileShaded(totoroMask(22, 19));
const GRID_IMP = compileShaded(totoroMask(24, 20));
const GRID_DAEMON = compileShaded(totoroMask(26, 21));
const GRID_ARCHON = compileShaded(totoroMask(28, 23));

function cellColor(code: CellCode, m: ColorMap): string | null {
  switch (code) {
    case "O":
      return m.outline;
    case "L":
      return m.light;
    case "B":
      return m.body;
    case "D":
      return m.dark;
    case "W":
      return m.belly;
    case "S":
      return m.light;
    case "H":
      return m.light;
    default:
      return null;
  }
}

function drawGrid(
  ctx: CanvasRenderingContext2D,
  ox: number,
  oy: number,
  grid: CellCode[][],
  m: ColorMap,
): void {
  for (let r = 0; r < grid.length; r++) {
    const row = grid[r];
    let c = 0;
    while (c < row.length) {
      const code = row[c];
      const color = cellColor(code, m);
      if (!color) {
        c++;
        continue;
      }
      let run = 1;
      while (c + run < row.length && row[c + run] === code) run++;
      ctx.fillStyle = color;
      ctx.fillRect(ox + c, oy + r, run, 1);
      c += run;
    }
  }
}

function faceOffset(anim: BuddyAnimState): number {
  return Math.round(anim.facingLerp * 1.6);
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
  fillPixel(
    ctx,
    ox + layout.noseX + off,
    oy + layout.noseY,
    layout.noseW,
    1,
    m.eyeDark,
  );
  fillPixel(
    ctx,
    ox + layout.noseX + ((layout.noseW / 2) | 0) + off,
    oy + layout.noseY + 1,
    1,
    1,
    m.eyeDark,
  );
  fillPixel(ctx, ox + layout.cheekLX + off, oy + layout.cheekY, 2, 1, m.rosy);
  fillPixel(ctx, ox + layout.cheekRX + off, oy + layout.cheekY, 2, 1, m.rosy);
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
  const lift = Math.round(Math.max(0, anim.earAnimProgress) * 2);
  const droop = anim.earAnimProgress < -0.3 ? 1 : 0;
  const twitch = anim.earTwitchTimer > 0 && anim.earTwitchTimer % 4 < 2 ? 1 : 0;
  const off = faceOffset(anim);
  const half = (earW / 2) | 0;

  const ear = (ecx: number, tw: number, lighter: boolean, tilt: number) => {
    const y = topY + droop - lift - tw;
    fillPixel(ctx, ecx - half + tilt, y - earH, earW, 1, m.outline);
    fillPixel(ctx, ecx - half - 1 + tilt, y - earH + 1, 1, earH - 1, m.outline);
    fillPixel(ctx, ecx + half + tilt, y - earH + 1, 1, earH - 1, m.outline);
    fillPixel(
      ctx,
      ecx - half + tilt,
      y - earH + 1,
      earW,
      earH,
      lighter ? m.light : m.body,
    );
    fillPixel(ctx, ecx - half + tilt, y - 1, earW, 1, m.dark);
    fillPixel(ctx, ecx - half + tilt, y - earH + 1, 1, 1, m.light);
  };

  ear(cx - spread + off, anim.earTwitchSide < 0 ? twitch : 0, true, 0);
  ear(cx + spread + off, anim.earTwitchSide > 0 ? twitch : 0, false, 0);
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
    fillPixel(ctx, bodyX - 2, anchorY + 3, 4, 2, m.body);
    fillPixel(ctx, bodyX - 3, anchorY + 4, 2, 1, m.dark);
    return;
  }
  const facingRight = anim.facingLerp >= 0;
  const x = facingRight ? bodyX - 4 : bodyX + bodyW - 1;
  const bob = Math.round(
    Math.sin(anim.tailPhase) * (0.4 + anim.tailEnergy * 0.9),
  );
  const sag = Math.round(anim.tailDroop * 2);
  const y = anchorY + sag - bob;
  fillPixel(ctx, x + 1, y, 3, 1, m.outline);
  fillPixel(ctx, x, y + 1, 1, 2, m.outline);
  fillPixel(ctx, x + 4, y + 1, 1, 2, m.outline);
  fillPixel(ctx, x + 1, y + 1, 3, 2, m.body);
  fillPixel(ctx, x + 1, y + 1, 2, 1, m.light);
  fillPixel(ctx, x + 1, y + 3, 3, 1, m.outline);
}

function drawLegs(
  ctx: CanvasRenderingContext2D,
  cx: number,
  footY: number,
  m: ColorMap,
  anim: BuddyAnimState,
): void {
  const foot = (x: number, y: number): void => {
    fillPixel(ctx, x, y - 1, 5, 2, m.body);
    fillPixel(ctx, x, y - 1, 2, 1, m.light);
    fillPixel(ctx, x, y + 1, 5, 1, m.dark);
    fillPixel(ctx, x + 1, y + 1, 1, 1, m.outline);
    fillPixel(ctx, x + 3, y + 1, 1, 1, m.outline);
  };
  if (anim.idleAction === "doze") {
    fillPixel(ctx, cx - 7, footY, 5, 1, m.dark);
    fillPixel(ctx, cx + 2, footY, 5, 1, m.dark);
    return;
  }
  if (anim.walking && Math.abs(anim.walkVel) > 0.08) {
    const liftA = Math.max(0, Math.sin(anim.walkPhase)) * 2.6;
    const liftB = Math.max(0, Math.sin(anim.walkPhase + Math.PI)) * 2.6;
    const strideA = Math.cos(anim.walkPhase) * 1.8 * anim.walkDirection;
    const strideB =
      Math.cos(anim.walkPhase + Math.PI) * 1.8 * anim.walkDirection;
    foot(Math.round(cx - 7 + strideA), Math.round(footY - liftA));
    foot(Math.round(cx + 2 + strideB), Math.round(footY - liftB));
    return;
  }
  if (anim.idleAction === "dance") {
    const hop = Math.sin(anim.dancePhase);
    foot(cx - 7, footY - Math.round(Math.max(0, hop) * 3));
    foot(cx + 2, footY - Math.round(Math.max(0, -hop) * 3));
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
    fillPixel(ctx, x, y, 2, 1, m.belly);
    fillPixel(ctx, x, y + 1, 1, 1, m.outline);
    fillPixel(ctx, x + 1, y + 1, 1, 1, m.outline);
  };
  const restArm = (x: number, y: number, outer: number): void => {
    fillPixel(ctx, x, y, 2, 5, m.body);
    fillPixel(ctx, outer, y, 1, 4, m.dark);
    fillPixel(ctx, x, y + 4, 2, 1, m.dark);
    fillPixel(ctx, x, y + 5, 1, 1, m.outline);
    fillPixel(ctx, x + 1, y + 5, 1, 1, m.outline);
  };

  switch (anim.armPose) {
    case "swing": {
      const sw = Math.round(Math.sin(anim.walkPhase) * 2);
      restArm(lx, midY - sw, lx - 1);
      restArm(rx, midY + sw, rx + 2);
      return;
    }
    case "wave": {
      const wx = dir > 0 ? rx : lx;
      const sx = dir > 0 ? lx : rx;
      const bob = Math.round(Math.sin(f * 0.28) * 1.5);
      restArm(sx, midY, dir > 0 ? sx - 1 : sx + 2);
      fillPixel(ctx, wx + dir, midY - 1, 2, 2, m.body);
      fillPixel(ctx, wx + dir * 2, midY - 3, 2, 2, m.body);
      fillPixel(ctx, wx + dir * 3, midY - 5 - bob, 2, 2, m.body);
      hand(wx + dir * 3, midY - 7 - bob);
      return;
    }
    case "raise": {
      const bounce = Math.round(Math.abs(Math.sin(f * 0.18)) * 2);
      fillPixel(ctx, lx - 1, midY - 3 - bounce, 2, 2, m.body);
      fillPixel(ctx, lx - 2, midY - 5 - bounce, 2, 2, m.body);
      hand(lx - 2, midY - 7 - bounce);
      fillPixel(ctx, rx + 1, midY - 3 - bounce, 2, 2, m.body);
      fillPixel(ctx, rx + 2, midY - 5 - bounce, 2, 2, m.body);
      hand(rx + 2, midY - 7 - bounce);
      return;
    }
    case "face": {
      const wx = dir > 0 ? rx : lx;
      const sx = dir > 0 ? lx : rx;
      restArm(sx, midY, dir > 0 ? sx - 1 : sx + 2);
      fillPixel(ctx, wx, midY - 1, 2, 2, m.body);
      fillPixel(ctx, wx - dir, midY - 3, 2, 2, m.body);
      hand(wx - dir, midY - 5);
      return;
    }
    case "drum": {
      const beat = Math.sin(f * 0.38) > 0 ? 1 : 0;
      fillPixel(ctx, lx + 3, midY + 3 + beat, 3, 2, m.body);
      hand(lx + 3, midY + 5 + beat);
      fillPixel(ctx, rx - 4, midY + 4 - beat, 3, 2, m.body);
      hand(rx - 4, midY + 6 - beat);
      return;
    }
    case "hold": {
      const wx = dir > 0 ? rx : lx;
      const sx = dir > 0 ? lx : rx;
      restArm(sx, midY, dir > 0 ? sx - 1 : sx + 2);
      fillPixel(ctx, wx + (dir > 0 ? 0 : -4), midY + 1, 5, 2, m.body);
      hand(wx + dir * 4, midY + 1);
      return;
    }
    case "hug": {
      fillPixel(ctx, lx + 3, midY + 2, 3, 2, m.body);
      fillPixel(ctx, rx - 4, midY + 2, 3, 2, m.body);
      hand(lx + 4, midY + 4);
      hand(rx - 4, midY + 4);
      return;
    }
    default: {
      const breath = Math.round(anim.breathScale * 90);
      restArm(lx, midY + breath, lx - 1);
      restArm(rx, midY + breath, rx + 2);
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
  const cx = ox + w / 2;
  const ruffle = 0.05 + anim.wingFlap * 0.12;
  const rows = [0.3, 0.46, 0.62, 0.78];
  for (let i = 0; i < rows.length; i++) {
    const r = Math.round(rows[i] * (h - 1));
    const ex = totoroRadius(w, h, r);
    const flick = Math.sin(anim.frame * ruffle + i * 1.9) > 0.55 ? 1 : 0;
    fillPixel(ctx, Math.round(cx - ex) - 1 - flick, oy + r, 1, 1, m.body);
    fillPixel(ctx, Math.round(cx + ex) + flick, oy + r, 1, 1, m.body);
    if (i % 2 === 0) {
      fillPixel(ctx, Math.round(cx - ex) - 1, oy + r + 1, 1, 1, m.dark);
      fillPixel(ctx, Math.round(cx + ex), oy + r + 1, 1, 1, m.dark);
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
  const gap = 5;
  const span = (count - 1) * gap + 3;
  for (let i = 0; i < count; i++) {
    const x = Math.round(cx - span / 2 + i * gap);
    fillPixel(ctx, x + 1, y, 1, 1, m.body);
    fillPixel(ctx, x, y + 1, 1, 1, m.body);
    fillPixel(ctx, x + 2, y + 1, 1, 1, m.body);
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
  const t = anim.earTwitchTimer > 0 && anim.earTwitchTimer % 4 < 2 ? 1 : 0;
  const off = faceOffset(anim);
  const L = lx + off;
  const R = rx + off;
  fillPixel(ctx, L - 3, y - 2 - t, 3, 1, m.outline);
  fillPixel(ctx, L - 4, y - 3 - t, 1, 1, m.outline);
  fillPixel(ctx, L - 4, y, 4, 1, m.outline);
  fillPixel(ctx, L - 3, y + 2, 3, 1, m.outline);
  fillPixel(ctx, L - 4, y + 3, 1, 1, m.outline);
  fillPixel(ctx, R, y - 2 - t, 3, 1, m.outline);
  fillPixel(ctx, R + 3, y - 3 - t, 1, 1, m.outline);
  fillPixel(ctx, R, y, 4, 1, m.outline);
  fillPixel(ctx, R, y + 2, 3, 1, m.outline);
  fillPixel(ctx, R + 3, y + 3, 1, 1, m.outline);
}

function drawLeafHat(
  ctx: CanvasRenderingContext2D,
  cx: number,
  topY: number,
  anim: BuddyAnimState,
): void {
  const sway = Math.round(Math.sin(anim.frame * 0.05) * 1.4);
  fillPixel(ctx, cx, topY - 1, 1, 2, "#3F6B35");
  fillPixel(ctx, cx - 2 + sway, topY - 3, 5, 2, "#5C9450");
  fillPixel(ctx, cx - 3 + sway, topY - 2, 2, 1, "#5C9450");
  fillPixel(ctx, cx + 3 + sway, topY - 4, 1, 1, "#5C9450");
  fillPixel(ctx, cx - 1 + sway, topY - 3, 3, 1, "#79B26A");
}

function drawLeafUmbrella(
  ctx: CanvasRenderingContext2D,
  cx: number,
  topY: number,
  anim: BuddyAnimState,
): void {
  const sway = Math.round(Math.sin(anim.frame * 0.04) * 1.5);
  fillPixel(ctx, cx, topY - 3, 1, 5, "#3F6B35");
  fillPixel(ctx, cx - 2 + sway, topY - 6, 5, 1, "#5C9450");
  fillPixel(ctx, cx - 5 + sway, topY - 5, 11, 1, "#5C9450");
  fillPixel(ctx, cx - 6 + sway, topY - 4, 13, 1, "#4A7D40");
  fillPixel(ctx, cx - 6 + sway, topY - 3, 2, 1, "#4A7D40");
  fillPixel(ctx, cx + 5 + sway, topY - 3, 2, 1, "#4A7D40");
  fillPixel(ctx, cx - 3 + sway, topY - 5, 4, 1, "#79B26A");
  fillPixel(ctx, cx + sway, topY - 6, 1, 1, "#79B26A");
}

function whiskerEdges(
  spec: TotoroSpec,
  ox: number,
): { lx: number; rx: number } {
  const cx = ox + (spec.w - 1) / 2;
  const radius = totoroRadius(spec.w, spec.h, spec.whiskerY);
  return {
    lx: Math.round(cx - radius) + 1,
    rx: Math.round(cx + radius),
  };
}

function armEdges(spec: TotoroSpec, ox: number): { lx: number; rx: number } {
  const cx = ox + (spec.w - 1) / 2;
  const radius = totoroRadius(spec.w, spec.h, spec.armY);
  return {
    lx: Math.round(cx - radius * 0.92),
    rx: Math.round(cx + radius * 0.92) - 1,
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
  drawGrid(ctx, ox, oy, spec.grid, m);
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
  grid: GRID_SPRITE,
  w: 22,
  h: 19,
  earSpread: 4,
  earH: 4,
  earW: 2,
  chevronRows: [[3, 10]],
  face: {
    eyeLX: 5,
    eyeRX: 14,
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
  whiskerY: 6,
  armY: 9,
};

const SPEC_IMP: TotoroSpec = {
  grid: GRID_IMP,
  w: 24,
  h: 20,
  earSpread: 4,
  earH: 5,
  earW: 2,
  chevronRows: [
    [3, 10],
    [2, 13],
  ],
  face: {
    eyeLX: 6,
    eyeRX: 15,
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
  whiskerY: 7,
  armY: 9,
};

const SPEC_DAEMON: TotoroSpec = {
  grid: GRID_DAEMON,
  w: 26,
  h: 21,
  earSpread: 5,
  earH: 6,
  earW: 2,
  chevronRows: [
    [4, 10],
    [3, 13],
  ],
  face: {
    eyeLX: 6,
    eyeRX: 17,
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
  whiskerY: 7,
  armY: 10,
};

const SPEC_ARCHON: TotoroSpec = {
  grid: GRID_ARCHON,
  w: 28,
  h: 23,
  earSpread: 5,
  earH: 6,
  earW: 2,
  chevronRows: [
    [4, 11],
    [3, 14],
  ],
  face: {
    eyeLX: 7,
    eyeRX: 18,
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
  whiskerY: 8,
  armY: 11,
};

const EGG_SPECKLES = [
  [6, 8],
  [13, 7],
  [9, 10],
  [5, 9],
  [14, 12],
  [8, 13],
] as const;

export function drawEgg(
  ctx: CanvasRenderingContext2D,
  ox: number,
  oy: number,
  m: ColorMap,
  anim: BuddyAnimState,
  _paletteIndex: number,
): void {
  const crack = Math.min(anim.frame / 30 / 10, 1);
  const rock = Math.round(Math.sin(anim.frame * 0.04) * 1.5);
  const x = ox + rock;
  drawGrid(ctx, x, oy, GRID_EGG, m);
  for (const [sx, sy] of EGG_SPECKLES) {
    fillPixel(ctx, x + sx, oy + sy, 1, 1, m.light);
  }

  for (let r = 0; r < 6; r++) {
    const row = GRID_EGG[r];
    for (let c = 0; c < row.length; c++) {
      if (row[c] === " ") continue;
      const zig = 4 + (c % 4 < 2 ? 1 : 0);
      if (r < zig) {
        fillPixel(ctx, x + c, oy + r, 1, 1, m.dark);
      } else if (r === zig) {
        fillPixel(ctx, x + c, oy + r, 1, 1, m.outline);
      }
    }
  }
  fillPixel(ctx, x + 9, oy - 2, 2, 2, m.outline);
  fillPixel(ctx, x + 9, oy - 3, 3, 1, m.dark);
  fillPixel(ctx, x + 5, oy + 1, 2, 1, m.light);

  if (crack > 0.1) {
    const d = Math.floor(crack * 8);
    const cx = x + 10;
    fillPixel(ctx, cx, oy + 6, 1, 1, m.outline);
    if (d > 1) fillPixel(ctx, cx - 1, oy + 7, 1, 1, m.outline);
    if (d > 2) fillPixel(ctx, cx, oy + 8, 1, 1, m.outline);
    if (d > 3) fillPixel(ctx, cx + 1, oy + 9, 1, 1, m.outline);
    if (d > 4) fillPixel(ctx, cx, oy + 10, 1, 1, m.outline);
    if (d > 5) fillPixel(ctx, cx - 1, oy + 11, 1, 1, m.outline);
    if (d > 6) fillPixel(ctx, cx, oy + 12, 1, 1, m.outline);
    if (d > 7) fillPixel(ctx, cx + 1, oy + 13, 1, 1, m.outline);
  }
  if (crack > 0.5) {
    ctx.globalAlpha = Math.min(1, (crack - 0.5) * 3);
    fillPixel(ctx, x + 7, oy + 9, 2, 2, m.eyeDark);
    fillPixel(ctx, x + 13, oy + 9, 2, 2, m.eyeDark);
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

  drawGrid(ctx, bx, bodyY, GRID_HATCHLING, pale);

  const off = faceOffset(anim);
  const hatTilt = Math.round(Math.sin(anim.frame * 0.02) * 1);
  const hatX = bx + 3 + off + hatTilt;
  fillPixel(ctx, hatX, bodyY - 2, 10, 3, m.belly);
  fillPixel(ctx, hatX + 1, bodyY - 3, 8, 1, m.belly);
  fillPixel(ctx, hatX, bodyY + 1, 2, 1, m.belly);
  fillPixel(ctx, hatX + 3, bodyY + 1, 2, 1, m.belly);
  fillPixel(ctx, hatX + 6, bodyY + 1, 2, 1, m.belly);
  fillPixel(ctx, hatX + 9, bodyY + 1, 1, 1, m.belly);
  fillPixel(ctx, hatX + 1, bodyY - 2, 2, 1, m.white);
  drawTotoroEars(ctx, cx - 1, bodyY, 3, 3, 2, pale, anim);

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

  const shellY = oy + 18;
  fillPixel(ctx, ox + 2, shellY, 16, 4, m.belly);
  fillPixel(ctx, ox + 1, shellY + 1, 18, 3, m.belly);
  fillPixel(ctx, ox + 2, shellY - 1, 2, 1, m.belly);
  fillPixel(ctx, ox + 6, shellY - 1, 2, 1, m.belly);
  fillPixel(ctx, ox + 10, shellY - 1, 2, 1, m.belly);
  fillPixel(ctx, ox + 14, shellY - 1, 2, 1, m.belly);
  fillPixel(ctx, ox + 1, shellY + 3, 18, 1, m.light);
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

  const cx = ox + 13;
  const crownY = oy - 3;
  fillPixel(ctx, cx, crownY - 1, 1, 1, m.outline);
  fillPixel(ctx, cx - 1, crownY, 3, 1, m.dark);
  fillPixel(ctx, cx - 1, crownY + 1, 3, 2, m.gold);

  const glow = 0.18 + Math.sin(f * 0.07) * 0.1;
  ctx.globalAlpha = glow;
  fillPixel(ctx, cx - 2, oy + 13, 5, 3, m.gold);
  ctx.globalAlpha = 1;

  for (let i = 0; i < 4; i++) {
    const a = f * 0.02 + i * 1.57;
    ctx.globalAlpha = 0.5 + Math.sin(f * 0.04 + i) * 0.3;
    fillPixel(
      ctx,
      (ox + 14 + Math.cos(a) * 18) | 0,
      (oy + 11 + Math.sin(a) * 10) | 0,
      2,
      2,
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
