import { fillPixel, fillRect, strokeRect } from "./helpers";
import { spriteColorRecord } from "./colorMap";
import { drawEyes, drawMouth, drawEarOverlay } from "./eyes";
import type { BuddyAnimState, ColorMap } from "../types";
import { PALETTES } from "../constants";

function row(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  pattern: string,
  cmap: Record<string, string>,
): void {
  for (let i = 0; i < pattern.length; i++) {
    const ch = pattern[i];
    if (ch !== " " && cmap[ch]) {
      ctx.fillStyle = cmap[ch];
      ctx.fillRect(x + i, y, 1, 1);
    }
  }
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
  const rock = Math.round(Math.sin(anim.frame * 0.04) * 1.5);
  const cmap = spriteColorRecord(m);
  const rows = [
    "      OOOOOOOO      ",
    "    OOWWWWWWWWOO    ",
    "   OWWWWWWWWWWWWO   ",
    "  OWWWWWWWWWWWWWWO  ",
    "  OWWWWWWWWWWWWWWO  ",
    " OWWWWWWWWWWWWWWWWO ",
    " OWWWWWWWWWWWWWWWWO ",
    " OWWWWWWWWWWWWWWWWO ",
    " OWWWWWWWWWWWWWWWWO ",
    " OWWWWWWWWWWWWWWWWO ",
    " OWWWWWWWWWWWWWWWWO ",
    "  OWWWWWWWWWWWWWWO  ",
    "  OWWWWWWWWWWWWWWO  ",
    "   OWWWWWWWWWWWWO   ",
    "    OOWWWWWWWWOO    ",
    "      OOOOOOOO      ",
  ];
  for (let r = 0; r < rows.length; r++)
    row(ctx, ox + rock, oy + r, rows[r], cmap);

  if (crack > 0.1) {
    const d = Math.floor(crack * 8);
    const cx = ox + rock + 10;
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
    fillPixel(ctx, ox + rock + 7, oy + 7, 2, 2, m.eyeDark);
    fillPixel(ctx, ox + rock + 13, oy + 7, 2, 2, m.eyeDark);
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
  const cmap = spriteColorRecord(m);
  const shell = [
    "  OOO      OOO  ",
    "  OWWWO    OWWWO ",
    "  OWWWWOOOOOWWWO ",
    "   OWWWWWWWWWWO  ",
    "    OOOOOOOOOO   ",
  ];
  for (let r = 0; r < shell.length; r++)
    row(ctx, ox + 1, oy + 18 + r, shell[r], cmap);
  const body = [
    "       OOOOOO       ",
    "     OOBBBBBBOO     ",
    "    OBBBBBBBBBBBO   ",
    "   OBBBBBBBBBBBBO   ",
    "   OBBBBBBBBBBBBO   ",
    "  OBBBBBBBBBBBBBO   ",
    "  OBBWWWWWWWWBBBO   ",
    "  OBBWWWWWWWWBBBO   ",
    "  OBBBBBBBBBBBBO    ",
    "   OBBBBBBBBBBO     ",
    "    OBBBBBBBBBO     ",
    "     OOBBBBBOO      ",
    "       OOOOO        ",
  ];
  for (let r = 0; r < body.length; r++) row(ctx, ox, oy + 5 + r, body[r], cmap);
  drawEyes(ctx, ox + 6, oy + 10, ox + 13, oy + 10, m, 2, anim);
  drawEarOverlay(ctx, ox + 4, oy + 8, m, anim);
  fillPixel(ctx, ox + 5, oy + 13, 2, 1, m.rosy);
  fillPixel(ctx, ox + 15, oy + 13, 2, 1, m.rosy);
  drawMouth(ctx, ox + 9, oy + 14, m, 3, anim);
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
  const ep = Math.max(0, anim.earAnimProgress);
  const cmap = spriteColorRecord(m);
  fillPixel(ctx, ox + 4, oy + 1, 2, 1, m.body);
  fillPixel(
    ctx,
    ox + 5 + (ep > 0.3 ? -1 : 0),
    oy + (ep > 0.3 ? -1 : 0),
    1,
    1,
    m.body,
  );
  fillPixel(ctx, ox + 19, oy + 1, 2, 1, m.body);
  fillPixel(
    ctx,
    ox + 20 + (ep > 0.3 ? 1 : 0),
    oy + (ep > 0.3 ? -1 : 0),
    1,
    1,
    m.body,
  );
  const body = [
    "      OOOOOOOOOO      ",
    "    OOBBBBBBBBBBBOO   ",
    "   OBBBBBBBBBBBBBBBO  ",
    "   OBBBBBBBBBBBBBBBBO ",
    "  OBBBBBBBBBBBBBBBBBO ",
    "  OBBBBBBBBBBBBBBBBBO ",
    "  OBBBBWWWWWWWWBBBBBO ",
    "  OBBBBWWWWWWWWBBBBBO ",
    "  OBBBBBBBBBBBBBBBBO  ",
    "  OBBBBBBBBBBBBBBBBO  ",
    "   OBBBBBBBBBBBBBBBO  ",
    "   OBBBBBBBBBBBBBBO   ",
    "    OOBBBBBBBBBOO     ",
    "      OOO   OOO      ",
    "     OBO     OBO      ",
    "      O       O       ",
  ];
  for (let r = 0; r < body.length; r++) row(ctx, ox, oy + 2 + r, body[r], cmap);
  const wv = Math.sin(anim.frame * 0.08);
  fillPixel(ctx, ox + 6, oy + 16 + (wv > 0 ? 1 : 0), 2, 1, m.body);
  fillPixel(ctx, ox + 10, oy + 16 + (wv < 0 ? 1 : 0), 2, 1, m.body);
  fillPixel(ctx, ox + 14, oy + 16 + (wv > 0 ? 1 : 0), 2, 1, m.body);
  drawEyes(ctx, ox + 7, oy + 7, ox + 14, oy + 7, m, 3, anim);
  drawEarOverlay(ctx, ox + 4, oy + 3, m, anim);
  fillPixel(ctx, ox + 5, oy + 11, 2, 1, m.rosy);
  fillPixel(ctx, ox + 18, oy + 11, 2, 1, m.rosy);
  drawMouth(ctx, ox + 10, oy + 12, m, 3, anim);
  if (anim.quirkActive && anim.quirkType === "phase") ctx.globalAlpha = 1;
}

export function drawImp(
  ctx: CanvasRenderingContext2D,
  ox: number,
  oy: number,
  m: ColorMap,
  anim: BuddyAnimState,
): void {
  const hp = Math.round(anim.earAnimProgress * 2);
  fillPixel(ctx, ox + 3, oy - hp, 1, 1, m.dark);
  fillPixel(ctx, ox + 4, oy + 1 - hp, 1, 1, m.dark);
  fillPixel(ctx, ox + 5, oy + 2, 1, 1, m.dark);
  fillPixel(ctx, ox + 22, oy - hp, 1, 1, m.dark);
  fillPixel(ctx, ox + 21, oy + 1 - hp, 1, 1, m.dark);
  fillPixel(ctx, ox + 20, oy + 2, 1, 1, m.dark);
  const cmap = spriteColorRecord(m);
  const body = [
    "      OOOOOOOOOOOO     ",
    "    OOBBBBBBBBBBBBBOO  ",
    "   OBBBBBBBBBBBBBBBBBBO",
    "  OBBBBBBBBBBBBBBBBBBO ",
    "  OBBBBBBBBBBBBBBBBBBO ",
    "  OBBBBBBBBBBBBBBBBBO  ",
    "  OBBBBWWWWWWWWWBBBBO  ",
    "  OBBBBWWWWWWWWWBBBBO  ",
    "  OBBBBBBBBBBBBBBBBO   ",
    "   OBBBBBBBBBBBBBBBO   ",
    "   OBBBBBBBBBBBBBBO    ",
    "    OOBBBBBBBBBOO      ",
    "      OOO   OOO        ",
  ];
  for (let r = 0; r < body.length; r++) row(ctx, ox, oy + 3 + r, body[r], cmap);
  const tw = Math.sin(anim.frame * 0.06) * 2;
  fillPixel(ctx, ox + 23, oy + 12, 1, 1, m.body);
  fillPixel(ctx, ox + 24, oy + 11, 1, 1, m.body);
  fillPixel(ctx, ox + 25 + Math.round(tw), oy + 10, 2, 1, m.dark);
  for (let i = 0; i < 4; i++) {
    fillPixel(
      ctx,
      ox + 5 + i * 5,
      oy + 16 + ((i + Math.floor(anim.frame / 10)) % 2 ? 1 : 0),
      2,
      1,
      m.body,
    );
  }
  drawEyes(ctx, ox + 8, oy + 8, ox + 16, oy + 8, m, 3, anim);
  fillPixel(ctx, ox + 7, oy + 7, 3, 1, m.eyeDark);
  fillPixel(ctx, ox + 17, oy + 7, 3, 1, m.eyeDark);
  fillPixel(ctx, ox + 5, oy + 12, 2, 1, m.rosy);
  fillPixel(ctx, ox + 19, oy + 12, 2, 1, m.rosy);
  drawMouth(ctx, ox + 11, oy + 13, m, 4, anim);
}

export function drawDaemon(
  ctx: CanvasRenderingContext2D,
  ox: number,
  oy: number,
  m: ColorMap,
  anim: BuddyAnimState,
): void {
  const hp = Math.round(anim.earAnimProgress * 2);
  fillPixel(ctx, ox + 2, oy - hp, 2, 1, m.dark);
  fillPixel(ctx, ox + 3, oy + 1 - hp, 2, 1, m.dark);
  fillPixel(ctx, ox + 4, oy + 2, 2, 1, m.dark);
  fillPixel(ctx, ox + 24, oy - hp, 2, 1, m.dark);
  fillPixel(ctx, ox + 23, oy + 1 - hp, 2, 1, m.dark);
  fillPixel(ctx, ox + 22, oy + 2, 2, 1, m.dark);
  fillPixel(ctx, ox, oy + 10, 1, 1, m.gold);
  fillPixel(ctx, ox - 1, oy + 11, 3, 1, m.gold);
  fillPixel(ctx, ox, oy + 12, 1, 1, m.gold);
  const cmap = spriteColorRecord(m);
  const body = [
    "       OOOOOOOOOOOO      ",
    "     OOBBBBBBBBBBBBBOO   ",
    "    OBBBBBBBBBBBBBBBBBO  ",
    "   OBBBBBBBBBBBBBBBBBBBO ",
    "   OBBBBBBBBBBBBBBBBBBO  ",
    "  OBBBBBBBBBBBBBBBBBBBO  ",
    "  OBBBBWWWWWWWWWWBBBBO   ",
    "  OBBBBWWWWWWWWWWBBBBO   ",
    "  OBBBBBBBBBBBBBBBBBBO   ",
    "   OBBBBBBBBBBBBBBBBBO   ",
    "   OBBBBBBBBBBBBBBBBO    ",
    "    OOBBBBBBBBBBBBOO     ",
    "      OOOO   OOOO        ",
  ];
  for (let r = 0; r < body.length; r++)
    row(ctx, ox + 1, oy + 3 + r, body[r], cmap);
  fillPixel(ctx, ox + 25, oy + 12, 1, 1, m.body);
  fillPixel(ctx, ox + 26, oy + 11, 1, 1, m.body);
  fillPixel(ctx, ox + 27, oy + 10, 1, 1, m.body);
  const tw = Math.sin(anim.frame * 0.05) * 2;
  fillPixel(ctx, ox + 28 + Math.round(tw), oy + 9, 2, 1, m.dark);
  fillPixel(ctx, ox + 29 + Math.round(tw), oy + 8, 1, 1, m.gold);
  for (let i = 0; i < 5; i++) {
    fillPixel(
      ctx,
      ox + 4 + i * 4,
      oy + 16 + ((i + Math.floor(anim.frame / 8)) % 2 ? 1 : 0),
      3,
      1,
      m.body,
    );
  }
  drawEyes(ctx, ox + 9, oy + 8, ox + 17, oy + 8, m, 3, anim);
  fillPixel(ctx, ox + 6, oy + 12, 2, 1, m.rosy);
  fillPixel(ctx, ox + 21, oy + 12, 2, 1, m.rosy);
  drawMouth(ctx, ox + 12, oy + 13, m, 4, anim);
  if (anim.shadowClone) {
    ctx.globalAlpha = anim.shadowClone.alpha * 0.25;
    fillRect(
      ctx,
      anim.shadowClone.x,
      anim.shadowClone.y,
      22,
      14,
      PALETTES[0].dark,
    );
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
  drawDaemon(ctx, ox, oy, m, anim);
  strokeRect(ctx, ox + 7, oy + 7, 5, 4, m.accent);
  strokeRect(ctx, ox + 15, oy + 7, 5, 4, m.accent);
  fillPixel(ctx, ox + 12, oy + 9, 3, 1, m.accent);
  fillPixel(ctx, ox + 11, oy + 15, 1, 1, m.accent);
  fillPixel(ctx, ox + 13, oy + 15, 1, 1, m.accent);
  fillPixel(ctx, ox + 12, oy + 16, 1, 1, m.accent);
  if (anim.auraPulseIntensity > 0) {
    ctx.globalAlpha = anim.auraPulseIntensity * 0.3;
    const r = Math.round(12 + Math.sin(anim.frame * 0.05) * 3);
    strokeRect(ctx, ox + 13 - r, oy + 9 - r, r * 2, r * 2, m.gold);
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
    fillPixel(ctx, ox + 5 + i, oy - 1 + (i < 3 || i > 8 ? 1 : 0), 1, 1, m.gold);
  }
  ctx.globalAlpha = 1;
  drawSage(ctx, ox, oy + 2, m, anim);
  for (let i = 0; i < 4; i++) {
    const a = f * 0.02 + i * 1.57;
    ctx.globalAlpha = 0.5 + Math.sin(f * 0.04 + i) * 0.3;
    fillPixel(
      ctx,
      (ox + 14 + Math.cos(a) * 18) | 0,
      (oy + 10 + Math.sin(a) * 10) | 0,
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
