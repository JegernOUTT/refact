import { fillPixel, strokeArc, strokeEllipse } from "./helpers";
import { drawEyes, drawMouth, drawBrows } from "./eyes";
import type { BuddyAnimState, ColorMap } from "../types";
import { PALETTES } from "../constants";

type CellCode = "O" | "L" | "B" | "D" | "W" | "S" | "H" | " ";

interface FaceLayout {
  eyeLX: number;
  eyeRX: number;
  eyeY: number;
  eyeSize: number;
  mouthX: number;
  mouthY: number;
  mouthW: number;
  cheekLX: number;
  cheekRX: number;
  cheekY: number;
}

function blobMask(
  w: number,
  h: number,
  exponent: number,
  belly: { cx: number; cy: number; rx: number; ry: number } | null,
): string[] {
  const rows: string[] = [];
  const cx = (w - 1) / 2;
  const cy = (h - 1) / 2;
  const rx = w / 2;
  const ry = h / 2;
  for (let r = 0; r < h; r++) {
    let row = "";
    for (let c = 0; c < w; c++) {
      const nx = Math.abs((c - cx) / rx);
      const ny = Math.abs((r - cy) / ry);
      const inside = Math.pow(nx, exponent) + Math.pow(ny, exponent) <= 1.005;
      if (!inside) {
        row += " ";
        continue;
      }
      if (belly) {
        const bx = (c - belly.cx) / belly.rx;
        const by = (r - belly.cy) / belly.ry;
        if (bx * bx + by * by <= 1) {
          row += "W";
          continue;
        }
      }
      row += "X";
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
const GRID_HATCH = compileShaded(
  blobMask(20, 13, 2.4, { cx: 9.5, cy: 8.4, rx: 6, ry: 4 }),
);
const GRID_SPRITE = compileShaded(
  blobMask(24, 16, 2.6, { cx: 11.5, cy: 10.4, rx: 7.4, ry: 4.6 }),
);
const GRID_IMP = compileShaded(
  blobMask(26, 16, 2.6, { cx: 12.5, cy: 10.4, rx: 8, ry: 4.6 }),
);
const GRID_DAEMON = compileShaded(
  blobMask(28, 16, 2.7, { cx: 13.5, cy: 10.4, rx: 8.6, ry: 4.6 }),
);
const GRID_ARCHON = compileShaded(
  blobMask(28, 19, 2.7, { cx: 13.5, cy: 12.6, rx: 8.6, ry: 5 }),
);

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
      return m.belly;
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

function drawCatEars(
  ctx: CanvasRenderingContext2D,
  ox: number,
  oy: number,
  w: number,
  m: ColorMap,
  anim: BuddyAnimState,
): void {
  const lift = Math.round(Math.max(0, anim.earAnimProgress) * 2);
  const droop = anim.earAnimProgress < -0.3 ? 1 : 0;
  const twitch = anim.earTwitchTimer > 0 && anim.earTwitchTimer % 4 < 2 ? 1 : 0;
  const lt = anim.earTwitchSide < 0 ? twitch : 0;
  const rt = anim.earTwitchSide > 0 ? twitch : 0;
  const off = faceOffset(anim);
  const lx = ox + Math.round(w * 0.18) + off;
  const rx = ox + Math.round(w * 0.68) + off;
  const ly = oy + droop - lift - lt;
  const ry = oy + droop - lift - rt;

  fillPixel(ctx, lx + 1, ly - 3, 1, 1, m.outline);
  fillPixel(ctx, lx, ly - 2, 3, 1, m.body);
  fillPixel(ctx, lx - 1, ly - 1, 4, 1, m.body);
  fillPixel(ctx, lx + 1, ly - 2, 1, 1, m.rosy);
  fillPixel(ctx, lx - 1, ly, 5, 1, m.dark);

  fillPixel(ctx, rx + 1, ry - 3, 1, 1, m.outline);
  fillPixel(ctx, rx, ry - 2, 3, 1, m.body);
  fillPixel(ctx, rx, ry - 1, 4, 1, m.body);
  fillPixel(ctx, rx + 1, ry - 2, 1, 1, m.rosy);
  fillPixel(ctx, rx - 1, ry, 5, 1, m.dark);
}

function drawHorns(
  ctx: CanvasRenderingContext2D,
  ox: number,
  oy: number,
  w: number,
  m: ColorMap,
  anim: BuddyAnimState,
  big: boolean,
): void {
  const lift = Math.round(Math.max(0, anim.earAnimProgress));
  const off = faceOffset(anim);
  const lx = ox + Math.round(w * 0.16) + off;
  const rx = ox + Math.round(w * 0.78) + off;
  const y = oy - lift;

  fillPixel(ctx, lx + 1, y, 2, 1, m.dark);
  fillPixel(ctx, lx, y - 1, 2, 1, m.dark);
  fillPixel(ctx, lx - 1, y - 2, 2, 1, m.dark);
  fillPixel(ctx, rx, y, 2, 1, m.dark);
  fillPixel(ctx, rx + 1, y - 1, 2, 1, m.dark);
  fillPixel(ctx, rx + 2, y - 2, 2, 1, m.dark);
  if (big) {
    fillPixel(ctx, lx - 1, y - 3, 1, 1, m.gold);
    fillPixel(ctx, rx + 3, y - 3, 1, 1, m.gold);
    fillPixel(ctx, lx + 1, y - 1, 1, 1, m.light);
    fillPixel(ctx, rx + 1, y - 1, 1, 1, m.light);
  }
}

function drawTail(
  ctx: CanvasRenderingContext2D,
  anchorX: number,
  anchorY: number,
  m: ColorMap,
  anim: BuddyAnimState,
  stage: number,
): void {
  if (anim.idleAction === "doze") {
    fillPixel(ctx, anchorX - 6, anchorY + 3, 4, 2, m.body);
    fillPixel(ctx, anchorX - 8, anchorY + 2, 3, 2, m.dark);
    return;
  }
  const dir = anim.facingLerp >= 0 ? -1 : 1;
  const wag = Math.sin(anim.tailPhase) * (0.3 + anim.tailEnergy * 0.75);
  let angle = -0.35 + anim.tailDroop * 1.1;
  let x = anchorX + dir * 1.5;
  let y = anchorY;
  const tipPoints: { x: number; y: number }[] = [];
  for (let s = 0; s < 3; s++) {
    angle += wag * (0.4 + s * 0.32);
    x += dir * (2.3 - s * 0.4);
    y += Math.sin(angle) * 2.1;
    const px = Math.round(x);
    const py = Math.round(y);
    const size = s === 2 ? 2 : 3;
    fillPixel(ctx, px, py, size, size, s === 2 ? m.dark : m.body);
    if (s === 0) fillPixel(ctx, px, py - 1, 2, 1, m.light);
    tipPoints.push({ x: px, y: py });
  }
  const tip = tipPoints[2];
  if (stage === 3 || stage === 4) {
    fillPixel(ctx, tip.x + dir * 2, tip.y, 1, 1, m.dark);
    fillPixel(ctx, tip.x + dir * 3, tip.y - 1, 1, 1, m.dark);
    fillPixel(ctx, tip.x + dir * 3, tip.y + 1, 1, 1, m.dark);
    if (stage === 4) {
      const flick = anim.frame % 8 < 4 ? 0 : 1;
      fillPixel(ctx, tip.x + dir * 3, tip.y - flick, 1, 1, m.gold);
    }
  } else if (stage === 5) {
    fillPixel(ctx, tip.x + dir * 2, tip.y - 1, 2, 3, m.light);
    fillPixel(ctx, tip.x + dir * 2, tip.y, 2, 1, m.belly);
  } else if (stage === 6) {
    const pulse = anim.frame % 10 < 5 ? 0 : 1;
    fillPixel(ctx, tip.x + dir * 2, tip.y - pulse, 2, 2, m.gold);
  } else {
    fillPixel(ctx, tip.x + dir, tip.y, 2, 2, m.body);
    fillPixel(ctx, tip.x + dir, tip.y - 1, 2, 1, m.light);
  }
}

function drawWings(
  ctx: CanvasRenderingContext2D,
  cx: number,
  topY: number,
  halfW: number,
  m: ColorMap,
  anim: BuddyAnimState,
  energy: boolean,
): void {
  const flap = anim.wingFlap;
  const beat = Math.sin(anim.frame * 0.45) * flap;
  const span = 4 + Math.round(flap * 5);
  const lift = Math.round(beat * 3);
  const edge = energy ? m.accent : m.light;
  const fill = energy ? m.light : m.dark;

  for (let i = 0; i < span; i++) {
    const rise = Math.round(i * (0.5 + flap * 0.3)) - lift;
    const len = Math.max(1, 4 - Math.floor(i / 2));
    const lx = cx - halfW - i;
    const rx = cx + halfW + i;
    const y = topY + 5 - rise;
    fillPixel(ctx, lx, y, 1, len, i === span - 1 ? edge : fill);
    fillPixel(ctx, rx, y, 1, len, i === span - 1 ? edge : fill);
    if (i % 2 === 0 && i > 0) {
      fillPixel(ctx, lx, y - 1, 1, 1, edge);
      fillPixel(ctx, rx, y - 1, 1, 1, edge);
    }
  }
}

function drawLegs(
  ctx: CanvasRenderingContext2D,
  cx: number,
  footY: number,
  m: ColorMap,
  anim: BuddyAnimState,
): void {
  const leg = (x: number, y: number): void => {
    fillPixel(ctx, x, y - 2, 3, 2, m.body);
    fillPixel(ctx, x - 1, y, 4, 2, m.dark);
    fillPixel(ctx, x - 1, y, 1, 1, m.light);
  };
  if (anim.idleAction === "doze") {
    fillPixel(ctx, cx - 6, footY - 1, 4, 2, m.dark);
    fillPixel(ctx, cx + 3, footY - 1, 4, 2, m.dark);
    return;
  }
  if (anim.walking && Math.abs(anim.walkVel) > 0.08) {
    const liftA = Math.max(0, Math.sin(anim.walkPhase)) * 2.6;
    const liftB = Math.max(0, Math.sin(anim.walkPhase + Math.PI)) * 2.6;
    const strideA = Math.cos(anim.walkPhase) * 1.8 * anim.walkDirection;
    const strideB =
      Math.cos(anim.walkPhase + Math.PI) * 1.8 * anim.walkDirection;
    leg(Math.round(cx - 6 + strideA), Math.round(footY - liftA));
    leg(Math.round(cx + 3 + strideB), Math.round(footY - liftB));
    return;
  }
  if (anim.idleAction === "dance") {
    const hop = Math.sin(anim.dancePhase);
    leg(cx - 6, footY - Math.round(Math.max(0, hop) * 3));
    leg(cx + 3, footY - Math.round(Math.max(0, -hop) * 3));
    return;
  }
  const shift = Math.sin(anim.frame * 0.013);
  leg(cx - 6, footY - (shift > 0.6 ? 1 : 0));
  leg(cx + 3, footY - (shift < -0.6 ? 1 : 0));
}

function drawArms(
  ctx: CanvasRenderingContext2D,
  ox: number,
  _oy: number,
  w: number,
  midY: number,
  m: ColorMap,
  anim: BuddyAnimState,
): void {
  const f = anim.frame;
  const dir = anim.facingLerp >= 0 ? 1 : -1;
  const lx = ox - 1;
  const rx = ox + w - 1;
  const hand = (x: number, y: number): void => {
    fillPixel(ctx, x, y, 2, 1, m.belly);
  };

  switch (anim.armPose) {
    case "swing": {
      const sw = Math.round(Math.sin(anim.walkPhase) * 2);
      fillPixel(ctx, lx, midY - sw, 2, 4, m.body);
      fillPixel(ctx, lx, midY - sw + 3, 2, 1, m.dark);
      fillPixel(ctx, rx, midY + sw, 2, 4, m.body);
      fillPixel(ctx, rx, midY + sw + 3, 2, 1, m.dark);
      return;
    }
    case "wave": {
      const wx = dir > 0 ? rx : lx;
      const sx = dir > 0 ? lx : rx;
      const bob = Math.round(Math.sin(f * 0.28) * 1.5);
      fillPixel(ctx, sx, midY, 2, 4, m.body);
      fillPixel(ctx, wx + dir, midY - 1, 2, 2, m.body);
      fillPixel(ctx, wx + dir * 2, midY - 3, 2, 2, m.body);
      fillPixel(ctx, wx + dir * 3, midY - 5 - bob, 2, 2, m.body);
      hand(wx + dir * 3, midY - 6 - bob);
      return;
    }
    case "raise": {
      const bounce = Math.round(Math.abs(Math.sin(f * 0.18)) * 2);
      fillPixel(ctx, lx - 1, midY - 3 - bounce, 2, 2, m.body);
      fillPixel(ctx, lx - 2, midY - 5 - bounce, 2, 2, m.body);
      hand(lx - 2, midY - 6 - bounce);
      fillPixel(ctx, rx + 1, midY - 3 - bounce, 2, 2, m.body);
      fillPixel(ctx, rx + 2, midY - 5 - bounce, 2, 2, m.body);
      hand(rx + 2, midY - 6 - bounce);
      return;
    }
    case "face": {
      const wx = dir > 0 ? rx : lx;
      const sx = dir > 0 ? lx : rx;
      fillPixel(ctx, sx, midY, 2, 4, m.body);
      fillPixel(ctx, wx, midY - 1, 2, 2, m.body);
      fillPixel(ctx, wx - dir, midY - 3, 2, 2, m.body);
      hand(wx - dir, midY - 4);
      return;
    }
    case "drum": {
      const beat = Math.sin(f * 0.38) > 0 ? 1 : 0;
      fillPixel(ctx, ox + 3, midY + 3 + beat, 3, 2, m.body);
      hand(ox + 3, midY + 4 + beat);
      fillPixel(ctx, ox + w - 6, midY + 4 - beat, 3, 2, m.body);
      hand(ox + w - 6, midY + 5 - beat);
      return;
    }
    case "hold": {
      const wx = dir > 0 ? rx : lx;
      const sx = dir > 0 ? lx : rx;
      fillPixel(ctx, sx, midY, 2, 4, m.body);
      fillPixel(
        ctx,
        wx + (dir > 0 ? 0 : -4),
        midY + 1,
        dir > 0 ? 5 : 5,
        2,
        m.body,
      );
      hand(wx + dir * 4, midY + 1);
      return;
    }
    case "hug": {
      fillPixel(ctx, ox + 4, midY + 2, 3, 2, m.body);
      fillPixel(ctx, ox + w - 7, midY + 2, 3, 2, m.body);
      hand(ox + 6, midY + 3);
      hand(ox + w - 8, midY + 3);
      return;
    }
    default: {
      const breath = Math.round(anim.breathScale * 90);
      fillPixel(ctx, lx, midY + breath, 2, 4, m.body);
      fillPixel(ctx, lx, midY + breath + 3, 2, 1, m.dark);
      fillPixel(ctx, rx, midY + breath, 2, 4, m.body);
      fillPixel(ctx, rx, midY + breath + 3, 2, 1, m.dark);
    }
  }
}

function edgeX(w: number, h: number, exponent: number, row: number): number {
  const ny = Math.abs((row - (h - 1) / 2) / (h / 2));
  const inner = Math.max(0, 1 - Math.pow(ny, exponent));
  return (w / 2) * Math.pow(inner, 1 / exponent);
}

function drawFurTufts(
  ctx: CanvasRenderingContext2D,
  ox: number,
  oy: number,
  w: number,
  h: number,
  exponent: number,
  m: ColorMap,
  anim: BuddyAnimState,
): void {
  const cx = ox + w / 2;
  const rows = [0.3, 0.46, 0.62, 0.78];
  for (let i = 0; i < rows.length; i++) {
    const r = Math.round(rows[i] * (h - 1));
    const ex = edgeX(w, h, exponent, r);
    const flick = Math.sin(anim.frame * 0.05 + i * 1.9) > 0.55 ? 1 : 0;
    fillPixel(ctx, Math.round(cx - ex) - 1 - flick, oy + r, 1, 1, m.body);
    fillPixel(ctx, Math.round(cx + ex) + flick, oy + r, 1, 1, m.body);
    if (i % 2 === 0) {
      fillPixel(ctx, Math.round(cx - ex) - 1, oy + r + 1, 1, 1, m.dark);
      fillPixel(ctx, Math.round(cx + ex), oy + r + 1, 1, 1, m.dark);
    }
  }
}

function drawChestChevrons(
  ctx: CanvasRenderingContext2D,
  cx: number,
  y: number,
  m: ColorMap,
  count: number,
): void {
  const span = count * 5;
  for (let i = 0; i < count; i++) {
    const x = Math.round(cx - span / 2 + i * 5 + 1);
    fillPixel(ctx, x, y, 1, 1, m.light);
    fillPixel(ctx, x + 1, y + 1, 1, 1, m.light);
    fillPixel(ctx, x + 2, y, 1, 1, m.light);
  }
}

function drawWhiskers(
  ctx: CanvasRenderingContext2D,
  ox: number,
  oy: number,
  w: number,
  cheekY: number,
  m: ColorMap,
  anim: BuddyAnimState,
): void {
  const off = faceOffset(anim);
  const twitch = anim.earTwitchTimer > 0 && anim.earTwitchTimer % 4 < 2 ? 1 : 0;
  fillPixel(ctx, ox - 4 + off, oy + cheekY - 2 - twitch, 3, 1, m.outline);
  fillPixel(ctx, ox - 4 + off, oy + cheekY, 3, 1, m.outline);
  fillPixel(ctx, ox + w + 1 + off, oy + cheekY - 2 - twitch, 3, 1, m.outline);
  fillPixel(ctx, ox + w + 1 + off, oy + cheekY, 3, 1, m.outline);
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

const EGG_SPECKLES = [
  [6, 5],
  [13, 7],
  [9, 10],
  [5, 9],
  [14, 12],
  [8, 3],
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
  fillPixel(ctx, x + 6, oy + 3, 2, 1, m.belly);
  fillPixel(ctx, x + 5, oy + 4, 1, 2, m.belly);

  if (crack > 0.1) {
    const d = Math.floor(crack * 8);
    const cx = x + 10;
    fillPixel(ctx, cx, oy + 3, 1, 1, m.outline);
    if (d > 1) fillPixel(ctx, cx - 1, oy + 4, 1, 1, m.outline);
    if (d > 2) fillPixel(ctx, cx, oy + 5, 1, 1, m.outline);
    if (d > 3) fillPixel(ctx, cx + 1, oy + 6, 1, 1, m.outline);
    if (d > 4) fillPixel(ctx, cx, oy + 7, 1, 1, m.outline);
    if (d > 5) fillPixel(ctx, cx - 1, oy + 8, 1, 1, m.outline);
    if (d > 6) fillPixel(ctx, cx, oy + 9, 1, 1, m.outline);
    if (d > 7) fillPixel(ctx, cx + 1, oy + 10, 1, 1, m.outline);
  }
  if (crack > 0.5) {
    ctx.globalAlpha = Math.min(1, (crack - 0.5) * 3);
    fillPixel(ctx, x + 7, oy + 7, 2, 2, m.eyeDark);
    fillPixel(ctx, x + 13, oy + 7, 2, 2, m.eyeDark);
    ctx.globalAlpha = 1;
  }
}

export function drawHatch(
  ctx: CanvasRenderingContext2D,
  ox: number,
  oy: number,
  m: ColorMap,
  anim: BuddyAnimState,
): void {
  const bodyY = oy + 6;
  drawTail(ctx, ox + 3, bodyY + 8, m, anim, 1);
  drawGrid(ctx, ox, bodyY, GRID_HATCH, m);
  drawFurTufts(ctx, ox, bodyY, 20, 13, 2.4, m, anim);
  drawLeafHat(ctx, ox + 10, bodyY - 4, anim);

  const off = faceOffset(anim);
  const hatTilt = Math.round(Math.sin(anim.frame * 0.02) * 1);
  const hatX = ox + 5 + off + hatTilt;
  fillPixel(ctx, hatX, bodyY - 3, 10, 3, m.belly);
  fillPixel(ctx, hatX + 1, bodyY - 4, 8, 1, m.belly);
  fillPixel(ctx, hatX, bodyY, 2, 1, m.belly);
  fillPixel(ctx, hatX + 3, bodyY, 2, 1, m.belly);
  fillPixel(ctx, hatX + 6, bodyY, 2, 1, m.belly);
  fillPixel(ctx, hatX + 9, bodyY, 1, 1, m.belly);
  fillPixel(ctx, hatX + 1, bodyY - 3, 2, 1, m.white);

  drawFace(ctx, ox, bodyY, m, anim, {
    eyeLX: 5,
    eyeRX: 12,
    eyeY: 4,
    eyeSize: 2,
    mouthX: 8,
    mouthY: 8,
    mouthW: 3,
    cheekLX: 3,
    cheekRX: 15,
    cheekY: 6,
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

  drawTail(ctx, ox + 3, oy + 10, m, anim, 2);
  drawCatEars(ctx, ox, oy + 1, 24, m, anim);
  drawGrid(ctx, ox, oy, GRID_SPRITE, m);
  drawFurTufts(ctx, ox, oy, 24, 16, 2.6, m, anim);
  drawChestChevrons(ctx, ox + 12, oy + 12, m, 3);
  drawLeafHat(ctx, ox + 12, oy, anim);
  drawWhiskers(ctx, ox, oy, 24, 8, m, anim);
  drawLegs(ctx, ox + 12, oy + 17, m, anim);
  drawFace(ctx, ox, oy, m, anim, {
    eyeLX: 6,
    eyeRX: 15,
    eyeY: 5,
    eyeSize: 3,
    mouthX: 10,
    mouthY: 10,
    mouthW: 3,
    cheekLX: 3,
    cheekRX: 19,
    cheekY: 8,
  });
  drawArms(ctx, ox, oy, 24, oy + 9, m, anim);

  if (anim.quirkActive && anim.quirkType === "phase") ctx.globalAlpha = 1;
}

export function drawImp(
  ctx: CanvasRenderingContext2D,
  ox: number,
  oy: number,
  m: ColorMap,
  anim: BuddyAnimState,
): void {
  drawTail(ctx, ox + 3, oy + 10, m, anim, 3);
  drawHorns(ctx, ox, oy + 1, 26, m, anim, false);
  drawGrid(ctx, ox, oy, GRID_IMP, m);
  drawFurTufts(ctx, ox, oy, 26, 16, 2.6, m, anim);
  drawChestChevrons(ctx, ox + 13, oy + 12, m, 3);
  drawWhiskers(ctx, ox, oy, 26, 8, m, anim);
  drawLegs(ctx, ox + 13, oy + 17, m, anim);
  drawFace(ctx, ox, oy, m, anim, {
    eyeLX: 7,
    eyeRX: 16,
    eyeY: 5,
    eyeSize: 3,
    mouthX: 11,
    mouthY: 10,
    mouthW: 4,
    cheekLX: 4,
    cheekRX: 21,
    cheekY: 8,
  });
  const off = faceOffset(anim);
  if (anim.moodType !== "concerned" && anim.idleAction !== "doze") {
    fillPixel(ctx, ox + 15 + off, oy + 11, 1, 1, m.white);
  }
  drawArms(ctx, ox, oy, 26, oy + 9, m, anim);
}

export function drawDaemon(
  ctx: CanvasRenderingContext2D,
  ox: number,
  oy: number,
  m: ColorMap,
  anim: BuddyAnimState,
): void {
  drawTail(ctx, ox + 3, oy + 10, m, anim, 4);
  drawWings(ctx, ox + 14, oy, 12, m, anim, false);
  drawHorns(ctx, ox, oy + 1, 28, m, anim, true);
  drawGrid(ctx, ox, oy, GRID_DAEMON, m);
  drawFurTufts(ctx, ox, oy, 28, 16, 2.7, m, anim);
  drawChestChevrons(ctx, ox + 14, oy + 12, m, 3);
  drawWhiskers(ctx, ox, oy, 28, 8, m, anim);
  drawLegs(ctx, ox + 14, oy + 17, m, anim);
  drawFace(ctx, ox, oy, m, anim, {
    eyeLX: 8,
    eyeRX: 17,
    eyeY: 5,
    eyeSize: 3,
    mouthX: 12,
    mouthY: 10,
    mouthW: 4,
    cheekLX: 5,
    cheekRX: 22,
    cheekY: 8,
  });
  drawArms(ctx, ox, oy, 28, oy + 9, m, anim);

  if (anim.shadowClone) {
    ctx.globalAlpha = anim.shadowClone.alpha * 0.25;
    ctx.fillStyle = PALETTES[0].dark;
    ctx.fillRect(anim.shadowClone.x | 0, anim.shadowClone.y | 0, 22, 14);
    ctx.globalAlpha = 1;
  }
}

export function drawSage(
  ctx: CanvasRenderingContext2D,
  ox: number,
  oy: number,
  m: ColorMap,
  anim: BuddyAnimState,
): void {
  drawTail(ctx, ox + 3, oy + 10, m, anim, 5);
  drawWings(ctx, ox + 14, oy, 12, m, anim, false);
  drawHorns(ctx, ox, oy + 1, 28, m, anim, true);
  drawGrid(ctx, ox, oy, GRID_DAEMON, m);
  drawFurTufts(ctx, ox, oy, 28, 16, 2.7, m, anim);
  drawChestChevrons(ctx, ox + 14, oy + 12, m, 3);
  drawLegs(ctx, ox + 14, oy + 17, m, anim);

  const off = faceOffset(anim);
  drawFace(ctx, ox, oy, m, anim, {
    eyeLX: 8,
    eyeRX: 17,
    eyeY: 5,
    eyeSize: 3,
    mouthX: 12,
    mouthY: 10,
    mouthW: 4,
    cheekLX: 5,
    cheekRX: 22,
    cheekY: 8,
  });

  const beardSway = Math.round(Math.sin(anim.frame * 0.04) * 1);
  fillPixel(ctx, ox + 10 + off, oy + 12, 8, 1, m.white);
  fillPixel(ctx, ox + 11 + off + beardSway, oy + 13, 6, 1, m.white);
  fillPixel(ctx, ox + 12 + off + beardSway, oy + 14, 4, 1, m.belly);

  strokeArc(
    ctx,
    ox + 9.5 + off,
    oy + 6.5,
    2.5,
    Math.PI * 0.05,
    Math.PI * 1.95,
    m.accent,
  );
  strokeArc(
    ctx,
    ox + 18.5 + off,
    oy + 6.5,
    2.5,
    Math.PI * 1.05,
    Math.PI * 2.95,
    m.accent,
  );
  fillPixel(ctx, ox + 12 + off, oy + 6, 4, 1, m.accent);
  fillPixel(ctx, ox + 13 + off, oy + 2, 2, 1, m.accent);
  fillPixel(ctx, ox + 13 + off, oy + 1, 2, 1, m.gold);

  drawArms(ctx, ox, oy, 28, oy + 9, m, anim);

  if (anim.auraPulseIntensity > 0) {
    ctx.globalAlpha = anim.auraPulseIntensity * 0.3;
    const r = 13 + Math.sin(anim.frame * 0.05) * 3;
    strokeEllipse(ctx, ox + 14, oy + 8, r, r * 0.72, m.gold);
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
  for (let i = 0; i < 12; i++) {
    ctx.globalAlpha = 0.4 + Math.sin(f * 0.06 + i * 0.7) * 0.35;
    fillPixel(ctx, ox + 8 + i, oy - 3 + (i < 3 || i > 8 ? 1 : 0), 1, 1, m.gold);
  }
  ctx.globalAlpha = 1;

  drawTail(ctx, ox + 3, oy + 12, m, anim, 6);
  drawWings(ctx, ox + 14, oy + 2, 12, m, anim, true);
  drawGrid(ctx, ox, oy, GRID_ARCHON, m);
  drawFurTufts(ctx, ox, oy, 28, 19, 2.7, m, anim);
  drawChestChevrons(ctx, ox + 14, oy + 16, m, 3);

  const crestPulse = Math.sin(f * 0.08) > 0 ? 0 : 1;
  fillPixel(ctx, ox + 13, oy - 2 - crestPulse, 2, 2, m.gold);
  fillPixel(ctx, ox + 9, oy - 1, 1, 2, m.gold);
  fillPixel(ctx, ox + 18, oy - 1, 1, 2, m.gold);

  fillPixel(ctx, ox + 6, oy + 4, 1, 4, m.light);
  fillPixel(ctx, ox + 7, oy + 8, 1, 3, m.light);
  fillPixel(ctx, ox + 21, oy + 6, 1, 4, m.light);
  fillPixel(ctx, ox + 20, oy + 10, 1, 3, m.light);

  drawLegs(ctx, ox + 14, oy + 20, m, anim);
  drawFace(ctx, ox, oy, m, anim, {
    eyeLX: 8,
    eyeRX: 17,
    eyeY: 6,
    eyeSize: 3,
    mouthX: 12,
    mouthY: 11,
    mouthW: 4,
    cheekLX: 5,
    cheekRX: 22,
    cheekY: 9,
  });

  const corePulse = 0.55 + Math.sin(f * 0.1) * 0.3;
  ctx.globalAlpha = corePulse;
  fillPixel(ctx, ox + 13, oy + 13, 2, 2, m.accent);
  fillPixel(ctx, ox + 12, oy + 14, 1, 1, m.accent);
  fillPixel(ctx, ox + 15, oy + 14, 1, 1, m.accent);
  fillPixel(ctx, ox + 13, oy + 12, 2, 1, m.gold);
  ctx.globalAlpha = 1;

  drawArms(ctx, ox, oy + 2, 28, oy + 11, m, anim);

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
