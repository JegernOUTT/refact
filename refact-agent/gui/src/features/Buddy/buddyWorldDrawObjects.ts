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
  strokeBezier,
  strokeLine,
  strokeEllipse,
  toneColor,
  wave,
  worldObjects,
  worldPaletteHint,
  worldPhase,
  type DrawBuddyWorldBaseArgs,
} from "./buddyWorldDrawHelpers";
import { phaseTints } from "./buddyWorldDrawScenery";

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
  const { ctx, palette, world } = args;
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const frame = safeFrame(args.frame);
  const x = pctX(width, BUDDY_WORLD_HOME_HOTSPOT.x);
  const y = pctY(height, BUDDY_WORLD_HOME_HOTSPOT.y);
  const scale = args.compact ? 0.86 : 1;
  const night = world.phase === "night" || world.phase === "evening";
  const winter = world.season === "winter";
  const glow = night
    ? 0.3 + wave(frame, 32, 0, 0.08, args.reducedMotion)
    : 0.14;

  fillEllipse(ctx, x, y + 16 * scale, 36 * scale, 14 * scale, "#FBBF24", glow);
  fillEllipse(ctx, x, y + 28 * scale, 40 * scale, 7 * scale, "#1A2E20", 0.3);

  const pathGlow = 0.4 + wave(frame, 40, 0, 0.04, args.reducedMotion);
  for (let index = 0; index < 6; index += 1) {
    const stepX = x + index * 9 * scale + Math.sin(index * 1.7) * 4;
    const stepY = y + 32 * scale + index * 5 * scale;
    fillEllipse(
      ctx,
      stepX,
      stepY,
      8 - index * 0.45,
      3.4,
      "#9A8A78",
      pathGlow - index * 0.04,
    );
    fillEllipse(
      ctx,
      stepX - 1,
      stepY - 1,
      5 - index * 0.3,
      1.6,
      "#C3B5A2",
      (pathGlow - index * 0.04) * 0.7,
    );
  }

  fillPixelRect(
    ctx,
    x - 24 * scale,
    y - 1 * scale,
    48 * scale,
    28 * scale,
    "#F2E8D5",
  );
  fillPixelRect(
    ctx,
    x - 24 * scale,
    y + 21 * scale,
    48 * scale,
    6 * scale,
    "#D9CBB4",
  );
  fillPixelRect(
    ctx,
    x - 25 * scale,
    y + 27 * scale,
    50 * scale,
    4 * scale,
    "#8C8378",
  );
  fillPixelRect(
    ctx,
    x - 24 * scale,
    y - 1 * scale,
    2 * scale,
    28 * scale,
    "#8B5E3C",
    0.85,
  );
  fillPixelRect(
    ctx,
    x + 22 * scale,
    y - 1 * scale,
    2 * scale,
    28 * scale,
    "#8B5E3C",
    0.85,
  );
  fillPixelRect(
    ctx,
    x - 9 * scale,
    y - 1 * scale,
    2 * scale,
    27 * scale,
    "#8B5E3C",
    0.6,
  );
  fillPixelRect(
    ctx,
    x + 8 * scale,
    y - 1 * scale,
    2 * scale,
    27 * scale,
    "#8B5E3C",
    0.6,
  );
  fillPixelRect(
    ctx,
    x - 24 * scale,
    y + 9 * scale,
    48 * scale,
    2 * scale,
    "#8B5E3C",
    0.5,
  );

  fillPixelRect(
    ctx,
    x - 28 * scale,
    y - 9 * scale,
    56 * scale,
    8 * scale,
    "#C2563C",
  );
  fillPixelRect(
    ctx,
    x - 24 * scale,
    y - 16 * scale,
    48 * scale,
    8 * scale,
    "#C2563C",
  );
  fillPixelRect(
    ctx,
    x - 18 * scale,
    y - 23 * scale,
    36 * scale,
    8 * scale,
    "#B14B34",
  );
  fillPixelRect(
    ctx,
    x - 12 * scale,
    y - 29 * scale,
    24 * scale,
    7 * scale,
    "#A8432E",
  );
  fillPixelRect(
    ctx,
    x - 28 * scale,
    y - 2 * scale,
    56 * scale,
    2 * scale,
    "#9E4530",
  );
  fillPixelRect(
    ctx,
    x - 24 * scale,
    y - 10 * scale,
    48 * scale,
    2 * scale,
    "#9E4530",
    0.8,
  );
  fillPixelRect(
    ctx,
    x - 18 * scale,
    y - 17 * scale,
    36 * scale,
    2 * scale,
    "#9E4530",
    0.8,
  );
  fillPixelRect(
    ctx,
    x - 13 * scale,
    y - 30 * scale,
    26 * scale,
    2 * scale,
    palette.body,
    0.9,
  );
  if (winter) {
    fillPixelRect(
      ctx,
      x - 27 * scale,
      y - 11 * scale,
      54 * scale,
      3 * scale,
      "#F4F8FB",
      0.92,
    );
    fillPixelRect(
      ctx,
      x - 17 * scale,
      y - 24 * scale,
      34 * scale,
      3 * scale,
      "#F4F8FB",
      0.9,
    );
  }

  fillPixelRect(
    ctx,
    x + 9 * scale,
    y - 42 * scale,
    9 * scale,
    16 * scale,
    "#A8968A",
  );
  fillPixelRect(
    ctx,
    x + 9 * scale,
    y - 42 * scale,
    3 * scale,
    16 * scale,
    "#8C7A6E",
  );
  fillPixelRect(
    ctx,
    x + 7 * scale,
    y - 44 * scale,
    13 * scale,
    3 * scale,
    "#6E6157",
  );
  if (!args.reducedMotion) {
    for (let puff = 0; puff < 3; puff += 1) {
      const rise = ((frame * 0.5 + puff * 34) % 100) / 100;
      const px =
        x + 13 * scale + rise * 14 + wave(frame, 26, puff * 2, 2, false);
      const py = y - 46 * scale - rise * 26;
      fillCircle(ctx, px, py, 3 + rise * 5, "#D8D3CA", (1 - rise) * 0.32);
    }
  }

  const windowLit = night;
  fillPixelRect(
    ctx,
    x - 3 * scale,
    y - 24 * scale,
    7 * scale,
    7 * scale,
    "#6E6157",
  );
  fillCircle(
    ctx,
    x + 0.5 * scale,
    y - 20.5 * scale,
    3 * scale,
    windowLit ? "#FFE9A8" : "#B9D4E4",
    0.95,
  );
  fillPixelRect(ctx, x - 0.5, y - 23 * scale, 1.4, 6 * scale, "#6E6157", 0.9);

  fillPixelRect(
    ctx,
    x + 11 * scale,
    y + 3 * scale,
    10 * scale,
    10 * scale,
    "#7A6A56",
  );
  fillPixelRect(
    ctx,
    x + 12 * scale,
    y + 4 * scale,
    8 * scale,
    8 * scale,
    windowLit ? "#FFE9A8" : "#BFD9E8",
  );
  fillPixelRect(
    ctx,
    x + 15.4 * scale,
    y + 4 * scale,
    1.4,
    8 * scale,
    "#7A6A56",
  );
  fillPixelRect(
    ctx,
    x + 12 * scale,
    y + 7.4 * scale,
    8 * scale,
    1.4,
    "#7A6A56",
  );
  if (windowLit) {
    fillCircle(ctx, x + 16 * scale, y + 8 * scale, 9 * scale, "#FFE9A8", 0.14);
  }
  fillPixelRect(
    ctx,
    x + 10 * scale,
    y + 13 * scale,
    12 * scale,
    2.4 * scale,
    "#6E5A44",
  );
  fillPixelRect(ctx, x + 11 * scale, y + 12 * scale, 2.4, 2, "#3E7C4F");
  fillPixelRect(ctx, x + 14 * scale, y + 11.4 * scale, 2.4, 2, "#4A8C58");
  fillPixelRect(ctx, x + 17 * scale, y + 12 * scale, 2.4, 2, "#3E7C4F");
  fillPixelRect(ctx, x + 12.4 * scale, y + 10.6 * scale, 1.6, 1.6, "#E981A0");
  fillPixelRect(ctx, x + 15.6 * scale, y + 10 * scale, 1.6, 1.6, "#F4B8C4");
  fillPixelRect(ctx, x + 18 * scale, y + 10.8 * scale, 1.6, 1.6, "#E981A0");

  fillPixelRect(
    ctx,
    x - 8 * scale,
    y + 5 * scale,
    14 * scale,
    24 * scale,
    "#92400E",
  );
  fillPixelRect(
    ctx,
    x - 7 * scale,
    y + 6 * scale,
    12 * scale,
    22 * scale,
    "#7C3A12",
  );
  fillPixelRect(ctx, x - 6 * scale, y + 7 * scale, 10 * scale, 1.6, "#A0522D");
  fillPixelRect(ctx, x - 6 * scale, y + 11 * scale, 10 * scale, 1.6, "#A0522D");
  fillCircle(
    ctx,
    x - 3.4 * scale,
    y + 17 * scale,
    1.3 * scale,
    "#FDE68A",
    0.95,
  );
  fillPixelRect(
    ctx,
    x - 4 * scale,
    y - 4 * scale,
    8 * scale,
    3 * scale,
    "#8B5E3C",
  );
  fillEllipse(
    ctx,
    x - 1 * scale,
    y + 30 * scale,
    9 * scale,
    2.4 * scale,
    "#B7AA96",
    0.9,
  );

  fillPixelRect(
    ctx,
    x - 26 * scale,
    y - 43 * scale,
    52 * scale,
    12 * scale,
    "#5C4A3A",
    0.92,
  );
  fillPixelRect(
    ctx,
    x - 23 * scale,
    y - 40 * scale,
    46 * scale,
    2 * scale,
    palette.body,
  );
  if (!args.compact) drawPixelText(ctx, "HOME", x, y - 36 * scale, "#F6EBDB");
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

interface ObjectMaterials {
  stone: string;
  stoneLight: string;
  stoneDark: string;
  wood: string;
  woodDark: string;
  paper: string;
  paperShade: string;
  moss: string;
}

function objectMaterials(args: DrawBuddyWorldBaseArgs): ObjectMaterials {
  const hint = worldPaletteHint(args.world);
  if (hint === "night" || hint === "dream") {
    return {
      stone: "#5C6678",
      stoneLight: "#76819A",
      stoneDark: "#3E4658",
      wood: "#4A3A2C",
      woodDark: "#32281E",
      paper: "#C9CFE2",
      paperShade: "#9AA3C0",
      moss: "#2C5638",
    };
  }
  if (hint === "storm") {
    return {
      stone: "#7A8494",
      stoneLight: "#97A1B0",
      stoneDark: "#566070",
      wood: "#5C4A38",
      woodDark: "#41342A",
      paper: "#D8DCE4",
      paperShade: "#AEB6C2",
      moss: "#33543A",
    };
  }
  if (hint === "dusk") {
    return {
      stone: "#9A8D95",
      stoneLight: "#B9A8AC",
      stoneDark: "#6E646E",
      wood: "#6E523C",
      woodDark: "#4C3A2C",
      paper: "#F6DFC8",
      paperShade: "#D9B49C",
      moss: "#3F6340",
    };
  }
  if (hint === "dawn") {
    return {
      stone: "#A89C92",
      stoneLight: "#C6B8AA",
      stoneDark: "#7A7066",
      wood: "#73573F",
      woodDark: "#52402F",
      paper: "#FBEEDC",
      paperShade: "#E4C7A8",
      moss: "#4A7449",
    };
  }
  return {
    stone: "#A8998A",
    stoneLight: "#C3B5A2",
    stoneDark: "#7A6E62",
    wood: "#6B4F3A",
    woodDark: "#4A362A",
    paper: "#FBF3E2",
    paperShade: "#E2CFAE",
    moss: "#4F8F54",
  };
}

function canopyFleck(args: DrawBuddyWorldBaseArgs): string {
  switch (worldPaletteHint(args.world)) {
    case "day":
      return "#BBF7D0";
    case "dawn":
      return "#D9EAC8";
    case "dusk":
      return "#FDBA74";
    case "night":
      return "#A7F3D0";
    case "dream":
      return "#C4B5FD";
    case "storm":
      return "#C2CEDC";
  }
}

function drawTaskGrove(
  args: DrawBuddyWorldBaseArgs,
  item: BuddyWorldObject,
  x: number,
  y: number,
  pulse: number,
  tone: string,
): void {
  const tints = phaseTints(args);
  const fleck = canopyFleck(args);
  const restless = item.state !== "calm";

  fillEllipse(args.ctx, x, y + 26, 26, 6, "#1A2E20", 0.28);
  fillPixelRect(args.ctx, x - 4, y + 6, 5, 21, tints.trunkDark, 0.96);
  fillPixelRect(args.ctx, x + 1, y + 6, 4, 21, tints.trunkLight, 0.96);
  fillPixelRect(args.ctx, x - 8, y + 23, 6, 4, tints.trunkDark, 0.88);
  fillPixelRect(args.ctx, x + 4, y + 23, 7, 4, tints.trunkLight, 0.84);

  fillEllipse(args.ctx, x - 11, y + 2 + pulse * 0.4, 15, 10, tints.canopyDeep);
  fillEllipse(args.ctx, x + 10, y + pulse * 0.5, 14, 9, tints.canopyDeep);
  fillEllipse(args.ctx, x - 2, y - 8 + pulse, 16, 10, tints.canopyMid);
  fillEllipse(args.ctx, x - 12, y - 9 + pulse, 9, 6, tints.canopyMid);
  fillEllipse(args.ctx, x + 8, y - 13 + pulse, 12, 8, tints.canopyLight);

  fillPixelRect(args.ctx, x - 6, y - 13 + pulse, 2, 2, fleck, 0.85);
  fillPixelRect(args.ctx, x + 12, y - 8 + pulse, 2, 2, fleck, 0.7);
  fillPixelRect(args.ctx, x + 2, y - 17 + pulse, 2, 2, fleck, 0.8);

  if (restless) {
    fillPixelRect(args.ctx, x - 9, y - 4 + pulse, 3, 3, tone, 0.92);
    fillPixelRect(args.ctx, x + 6, y - 9 + pulse, 3, 3, tone, 0.92);
    fillPixelRect(args.ctx, x + 13, y + 2 + pulse * 0.5, 3, 3, tone, 0.88);
    fillPixelRect(args.ctx, x + 16, y + 25, 3, 3, tone, 0.66);
  }
}

function drawMemoryFireflies(
  args: DrawBuddyWorldBaseArgs,
  item: BuddyWorldObject,
  x: number,
  y: number,
  tone: string,
): void {
  const materials = objectMaterials(args);
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

  const swing = wave(args.frame, 64, 1, 1.4, args.reducedMotion);
  strokeLine(
    args.ctx,
    { x: x + swing * 0.3, y: y + 6 },
    { x: x + 5, y: y - 18 },
    materials.woodDark,
    1.2,
    0.8,
  );
  fillEllipse(args.ctx, x + swing, y + 15, 8, 9, materials.wood, 0.96);
  fillEllipse(args.ctx, x + swing, y + 11, 7, 4, materials.moss, 0.9);
  fillPixelRect(
    args.ctx,
    x - 6 + swing,
    y + 14,
    12,
    1.6,
    materials.woodDark,
    0.5,
  );
  fillPixelRect(
    args.ctx,
    x - 5 + swing,
    y + 18,
    10,
    1.4,
    materials.woodDark,
    0.4,
  );
  fillEllipse(args.ctx, x + swing, y + 16, 3.4, 4, "#1F1812", 0.92);
  fillPixelRect(args.ctx, x - 1.4 + swing, y + 14.6, 3, 3, glowColor, 0.9);
  fillEllipse(args.ctx, x, y + 26, 12, 3, "#1A2E20", 0.3);
}

function drawObservatory(
  args: DrawBuddyWorldBaseArgs,
  item: BuddyWorldObject,
  x: number,
  y: number,
  tone: string,
): void {
  const materials = objectMaterials(args);
  const tints = phaseTints(args);
  const frame = safeFrame(args.frame);
  const phase = worldPhase(args.world);
  const lanternLit = phase === "evening" || phase === "night";
  const activeAlpha =
    item.state === "critical" ? 0.32 : item.state === "active" ? 0.2 : 0.1;
  const warning = item.state === "attention";

  fillCircle(args.ctx, x, y - 14, 25, tone, activeAlpha);
  fillEllipse(args.ctx, x + 2, y + 24, 36, 9, tints.canopyDeep, 0.88);
  fillEllipse(args.ctx, x - 2, y + 20, 28, 8, tints.canopyMid, 0.9);
  fillEllipse(args.ctx, x + 14, y + 23, 12, 4, tints.canopyLight, 0.5);

  const lx = x - 9;
  fillPixelRect(args.ctx, lx - 7, y + 14, 15, 3, materials.stoneDark, 0.95);
  fillPixelRect(args.ctx, lx - 5, y + 11, 11, 3, materials.stone, 0.95);
  fillPixelRect(args.ctx, lx - 3, y + 2, 7, 9, materials.stone, 0.96);
  fillPixelRect(args.ctx, lx - 3, y + 2, 2, 9, materials.stoneDark, 0.5);
  fillPixelRect(args.ctx, lx - 6, y - 5, 13, 7, materials.stoneLight, 0.97);
  fillPixelRect(
    args.ctx,
    lx - 4,
    y - 3.4,
    9,
    4.4,
    lanternLit ? "#FDE68A" : materials.stoneDark,
    lanternLit ? 0.95 : 0.85,
  );
  if (lanternLit) {
    fillCircle(
      args.ctx,
      lx,
      y - 1,
      11,
      "#FBBF24",
      alphaForMotion(
        0.12 + wave(frame, 30, 1, 0.04, args.reducedMotion),
        args.reducedMotion,
      ),
    );
  }
  fillPixelRect(args.ctx, lx - 8, y - 8, 17, 3, materials.stoneDark, 0.96);
  fillPixelRect(args.ctx, lx - 5, y - 10.4, 11, 2.6, materials.stone, 0.96);
  fillPixelRect(args.ctx, lx - 1.4, y - 13, 3, 3, materials.stoneDark, 0.96);

  const tx = x + 10;
  strokeLine(
    args.ctx,
    { x: tx - 4, y: y + 14 },
    { x: tx + 1, y: y + 3 },
    materials.woodDark,
    1.6,
    0.92,
  );
  strokeLine(
    args.ctx,
    { x: tx + 7, y: y + 14 },
    { x: tx + 1, y: y + 3 },
    materials.woodDark,
    1.6,
    0.92,
  );
  fillPixelRect(args.ctx, tx - 3, y - 1, 9, 4, materials.woodDark, 0.96);
  fillPixelRect(args.ctx, tx + 4, y - 4.4, 7, 4.4, materials.wood, 0.96);
  fillPixelRect(args.ctx, tx + 10, y - 4.4, 2.4, 4.4, "#DBEAFE", 0.9);

  if (item.state === "active") {
    strokeLine(
      args.ctx,
      { x: tx + 11, y: y - 3 },
      {
        x: tx + 34,
        y: y - 36 + wave(frame, 58, 0, 6, args.reducedMotion),
      },
      "#DBEAFE",
      2.4,
      0.26 + finiteOr(item.intensity, 0) * 0.18,
    );
  }
  if (warning) {
    strokeEllipse(
      args.ctx,
      x,
      y - 2,
      30,
      19,
      "#F59E0B",
      2,
      0.14 + finiteOr(item.intensity, 0) * 0.08,
    );
  }
  if (item.state === "critical") {
    fillCircle(args.ctx, x, y - 8, 32, "#EF4444", 0.13);
    fillPixelRect(args.ctx, x + 21, y - 22, 8, 3, "#FACC15", 0.86);
    fillPixelRect(args.ctx, x + 26, y - 19, 3, 8, "#FACC15", 0.86);
    strokeLine(
      args.ctx,
      { x: x + 22, y: y - 21 },
      { x: x + 46, y: y - 48 },
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
  const materials = objectMaterials(args);
  const frame = safeFrame(args.frame);
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const ax = pctX(width, finiteOr(item.interactionX, 78));
  const ay = pctY(height, finiteOr(item.interactionY, 72)) + 10;
  const sway = wave(frame, 46, 0, 4, args.reducedMotion);
  const kx = x + sway;
  const ky = y + pulse;

  strokeBezier(
    args.ctx,
    { x: kx, y: ky + 9 },
    { x: kx - 12, y: ky + 34 },
    { x: ax + 16, y: ay - 36 },
    { x: ax, y: ay - 8 },
    materials.paperShade,
    1,
    0.6,
  );
  fillPixelRect(args.ctx, ax - 1.4, ay - 9, 3, 11, materials.woodDark, 0.95);
  fillPixelRect(args.ctx, ax - 3.4, ay + 1, 7, 2.4, materials.stoneDark, 0.9);

  const rows = [2, 6, 10, 13, 10, 6, 2];
  for (let row = 0; row < rows.length; row += 1) {
    const rowWidth = rows[row];
    fillPixelRect(
      args.ctx,
      kx - rowWidth / 2,
      ky - 9 + row * 2.6,
      rowWidth,
      2.6,
      row === 3 ? tone : row < 3 ? materials.paper : materials.paperShade,
      0.96,
    );
  }
  fillPixelRect(args.ctx, kx - 0.8, ky - 9, 1.6, 18.2, materials.woodDark, 0.6);
  fillPixelRect(args.ctx, kx - 6.5, ky - 0.8, 13, 1.6, materials.woodDark, 0.6);

  for (let bow = 0; bow < 3; bow += 1) {
    const bowSway = wave(frame, 16 + bow * 4, bow * 1.3, 3, args.reducedMotion);
    fillPixelRect(
      args.ctx,
      kx - 1.5 + bowSway,
      ky + 12 + bow * 6,
      3.4,
      2.2,
      bow % 2 === 0 ? tone : materials.paper,
      0.9 - bow * 0.14,
    );
  }
}

function drawGitVane(
  args: DrawBuddyWorldBaseArgs,
  x: number,
  y: number,
  tone: string,
): void {
  const materials = objectMaterials(args);
  const frame = safeFrame(args.frame);
  const sway = wave(frame, 34, 0, 2.4, args.reducedMotion);

  fillEllipse(args.ctx, x, y + 13, 10, 3, "#1A2E20", 0.3);
  fillPixelRect(args.ctx, x - 4, y + 9, 8, 3.4, materials.stone, 0.92);
  fillPixelRect(args.ctx, x - 1.6, y - 14, 3.4, 24, materials.woodDark, 0.95);
  fillPixelRect(args.ctx, x, y - 14, 1.6, 24, materials.wood, 0.9);

  fillPixelRect(
    args.ctx,
    x - 11 + sway,
    y - 11,
    22,
    2,
    materials.woodDark,
    0.94,
  );
  fillPixelRect(
    args.ctx,
    x + 11 + sway,
    y - 12.4,
    3,
    5,
    materials.woodDark,
    0.94,
  );
  fillPixelRect(
    args.ctx,
    x - 14 + sway,
    y - 13,
    3.4,
    2,
    materials.woodDark,
    0.9,
  );
  fillPixelRect(
    args.ctx,
    x - 14 + sway,
    y - 9,
    3.4,
    2,
    materials.woodDark,
    0.9,
  );

  const rx = x + sway * 0.6;
  fillPixelRect(args.ctx, rx - 2.4, y - 19, 5.4, 3.4, "#7C2D12", 0.95);
  fillPixelRect(args.ctx, rx - 4.4, y - 21.6, 2.4, 3.4, "#9A3412", 0.95);
  fillPixelRect(args.ctx, rx + 2.6, y - 21, 2.2, 2.2, "#7C2D12", 0.95);
  fillPixelRect(args.ctx, rx + 3, y - 22.4, 1.4, 1.4, tone, 0.95);
  fillPixelRect(args.ctx, rx + 4.6, y - 20.2, 1.4, 1.2, "#FDE68A", 0.95);

  fillPixelRect(args.ctx, x - 8, y - 2, 2, 2, materials.stone, 0.8);
  fillPixelRect(args.ctx, x + 6, y - 2, 2, 2, materials.stone, 0.8);
}

function drawMarketComet(
  args: DrawBuddyWorldBaseArgs,
  x: number,
  y: number,
  pulse: number,
): void {
  const frame = safeFrame(args.frame);
  const drift = wave(frame, 74, 1, 3, args.reducedMotion);
  const bx = x + drift;
  const by = y + pulse;

  fillEllipse(args.ctx, bx, by - 2, 11, 13, "#E981A0", 0.97);
  fillPixelRect(args.ctx, bx - 7.4, by - 8, 3.2, 13, "#FBEEDC", 0.85);
  fillPixelRect(args.ctx, bx - 1.6, by - 14, 3.2, 19, "#FBEEDC", 0.85);
  fillPixelRect(args.ctx, bx + 4.2, by - 8, 3.2, 13, "#FBEEDC", 0.85);
  fillEllipse(args.ctx, bx - 4, by - 8, 4, 5, "#F6C8D8", 0.55);
  fillPixelRect(args.ctx, bx - 8, by + 8, 16, 2.4, "#C98D96", 0.95);

  strokeLine(
    args.ctx,
    { x: bx - 6, y: by + 10 },
    { x: bx - 3, y: by + 17 },
    "#6B4F3A",
    1,
    0.85,
  );
  strokeLine(
    args.ctx,
    { x: bx + 6, y: by + 10 },
    { x: bx + 3, y: by + 17 },
    "#6B4F3A",
    1,
    0.85,
  );
  fillPixelRect(args.ctx, bx - 4.4, by + 17, 9, 5.4, "#8A6A4F", 0.96);
  fillPixelRect(args.ctx, bx - 4.4, by + 19.4, 9, 1.2, "#6B4F3A", 0.9);
  const flicker = Math.abs(wave(frame, 7, 0, 0.5, args.reducedMotion));
  fillPixelRect(
    args.ctx,
    bx - 1.2,
    by + 13.6,
    2.4,
    2.6,
    "#FDE68A",
    0.6 + flicker,
  );
}

function drawSeed(args: DrawBuddyWorldBaseArgs, x: number, y: number): void {
  const frame = safeFrame(args.frame);
  const sway = wave(frame, 40, 0, 1.6, args.reducedMotion);

  fillEllipse(args.ctx, x, y + 12, 13, 4.4, "#6B4F3A", 0.92);
  fillEllipse(args.ctx, x, y + 10.6, 10, 3, "#8A6A4F", 0.8);
  fillPixelRect(args.ctx, x - 1 + sway * 0.4, y - 2, 2.4, 13, "#2E7D45", 0.96);
  fillEllipse(args.ctx, x - 5 + sway, y - 4, 5.4, 3, "#74B06A", 0.95);
  fillEllipse(args.ctx, x + 5 + sway, y - 6, 5.4, 3, "#4F8F54", 0.95);
  fillPixelRect(args.ctx, x - 1 + sway, y - 7.4, 2.4, 2.4, "#86EFAC", 0.95);
  drawSpark(args.ctx, x + 9, y - 10, 1.6, "#FDE68A", 0.6);
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
