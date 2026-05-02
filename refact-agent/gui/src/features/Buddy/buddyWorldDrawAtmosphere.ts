import type { BuddyWorldState } from "./buddyWorldModel";
import {
  BUDDY_WORLD_HOME_HOTSPOT,
  alphaForMotion,
  clamp,
  countForMotion,
  drawCloud,
  drawPixelText,
  drawSpark,
  fillCircle,
  fillEllipse,
  fillPixelRect,
  fillRect,
  hasWorldLayer,
  lerp,
  objectAnchor,
  pctX,
  pctY,
  safeDimension,
  safeFrame,
  seededRange,
  seededUnit,
  strokeBezier,
  strokeCircle,
  strokeEllipse,
  strokeLine,
  TAU,
  wave,
  worldIntensity,
  worldObjects,
  worldPaletteHint,
  worldPhase,
  worldWeather,
  type DrawBuddyWorldBaseArgs,
} from "./buddyWorldDrawHelpers";

interface SkyStop {
  offset: number;
  color: string;
}

const SKY_STOPS: Record<
  BuddyWorldState["atmosphere"]["paletteHint"],
  SkyStop[]
> = {
  dawn: [
    { offset: 0, color: "#0EA5E9" },
    { offset: 0.5, color: "#F59E0B" },
    { offset: 1, color: "#166534" },
  ],
  day: [
    { offset: 0, color: "#38BDF8" },
    { offset: 0.58, color: "#93C5FD" },
    { offset: 1, color: "#15803D" },
  ],
  dusk: [
    { offset: 0, color: "#7C2D12" },
    { offset: 0.54, color: "#6D28D9" },
    { offset: 1, color: "#14532D" },
  ],
  night: [
    { offset: 0, color: "#111827" },
    { offset: 0.55, color: "#312E81" },
    { offset: 1, color: "#064E3B" },
  ],
  dream: [
    { offset: 0, color: "#1E1B4B" },
    { offset: 0.58, color: "#6D28D9" },
    { offset: 1, color: "#064E3B" },
  ],
  storm: [
    { offset: 0, color: "#020617" },
    { offset: 0.5, color: "#312E81" },
    { offset: 1, color: "#14532D" },
  ],
};

export function drawSkyGradient(args: DrawBuddyWorldBaseArgs): void {
  const { ctx } = args;
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const gradient = ctx.createLinearGradient(0, 0, 0, height);
  const stops = SKY_STOPS[worldPaletteHint(args.world)];

  for (const stop of stops) {
    gradient.addColorStop(clamp(stop.offset, 0, 1), stop.color);
  }

  fillRect(ctx, 0, 0, width, height, gradient);
}

function drawSkyStructures(args: DrawBuddyWorldBaseArgs): void {
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const frame = safeFrame(args.frame);
  const serious = args.world.atmosphere.serious;
  const warning = hasWorldLayer(args.world, "provider_flicker") && !serious;
  const active = hasWorldLayer(args.world, "workshop_runes");
  const crystalX = width * 0.75;
  const crystalY = height * 0.49;
  const lighthouseX = width * 0.88;
  const lighthouseY = height * 0.56;
  const crystalTone = serious ? "#F87171" : warning ? "#F59E0B" : "#93C5FD";
  const beaconTone = serious || warning ? "#FDE68A" : "#E0E7FF";
  const beamAlpha = alphaForMotion(
    serious ? 0.24 : active ? 0.17 : warning ? 0.12 : 0.1,
    args.reducedMotion,
  );

  fillCircle(
    args.ctx,
    crystalX,
    crystalY - 18,
    serious ? 32 : 24,
    crystalTone,
    serious ? 0.07 : 0.035,
  );
  fillPixelRect(
    args.ctx,
    crystalX - 6,
    crystalY - 34,
    12,
    20,
    crystalTone,
    0.8,
  );
  fillPixelRect(
    args.ctx,
    crystalX - 12,
    crystalY - 18,
    24,
    18,
    "#1E293B",
    0.86,
  );
  fillPixelRect(
    args.ctx,
    crystalX - 4,
    crystalY - 28,
    8,
    24,
    "#DBEAFE",
    serious ? 0.52 : 0.42,
  );
  fillPixelRect(
    args.ctx,
    crystalX + 10,
    crystalY - 28 + wave(frame, 36, 0, 3, args.reducedMotion),
    4,
    4,
    beaconTone,
    0.72,
  );
  if (warning || serious) {
    strokeCircle(
      args.ctx,
      crystalX,
      crystalY - 19,
      28 + wave(frame, warning ? 28 : 18, 0, 3, args.reducedMotion),
      warning ? "#FCD34D" : "#F87171",
      warning ? 1.5 : 3,
      alphaForMotion(warning ? 0.12 : 0.32, args.reducedMotion),
    );
  }

  fillPixelRect(
    args.ctx,
    lighthouseX - 10,
    lighthouseY - 26,
    20,
    44,
    "#334155",
    0.92,
  );
  fillPixelRect(
    args.ctx,
    lighthouseX - 15,
    lighthouseY + 15,
    30,
    6,
    "#0F172A",
    0.72,
  );
  fillPixelRect(
    args.ctx,
    lighthouseX - 7,
    lighthouseY - 36,
    14,
    10,
    beaconTone,
    0.86,
  );
  fillPixelRect(
    args.ctx,
    lighthouseX - 12,
    lighthouseY - 40,
    24,
    5,
    "#CBD5E1",
    0.9,
  );
  strokeLine(
    args.ctx,
    { x: lighthouseX - 7, y: lighthouseY - 31 },
    {
      x: width * 0.58,
      y: height * 0.18 + wave(frame, 86, 0, 5, args.reducedMotion),
    },
    beaconTone,
    args.compact ? 2 : 3,
    beamAlpha,
  );
  if (active) {
    strokeLine(
      args.ctx,
      { x: crystalX + 8, y: crystalY - 25 },
      {
        x: width * 0.48 + wave(frame, 72, 0, 14, args.reducedMotion),
        y: height * 0.18 + wave(frame, 58, 1, 5, args.reducedMotion),
      },
      "#22D3EE",
      args.compact ? 2 : 4,
      alphaForMotion(0.22, args.reducedMotion),
    );
  }
}

export function shouldDrawStarField(world: BuddyWorldState): boolean {
  return hasWorldLayer(world, "stars");
}

export function drawStarField(args: DrawBuddyWorldBaseArgs): void {
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const phase = worldPhase(args.world);
  const hint = worldPaletteHint(args.world);
  const frame = safeFrame(args.frame);
  const starCount = countForMotion(
    hint === "storm" ? 72 : 54,
    args.compact,
    args.reducedMotion,
  );
  const starAlpha =
    hint === "night" || hint === "dream" || hint === "storm"
      ? 0.72
      : phase === "evening"
        ? 0.36
        : 0.18;

  for (let index = 0; index < starCount; index += 1) {
    const x = (seededUnit(19, index) * width + frame * 0.035) % width;
    const y = seededUnit(29, index) * height * 0.52;
    const size = index % 7 === 0 ? 3 : index % 5 === 0 ? 2.4 : 1.8;
    const twinkle = args.reducedMotion
      ? 0
      : Math.sin(frame / 42 + index) * 0.12;
    fillPixelRect(
      args.ctx,
      x,
      y,
      size,
      size,
      index % 11 === 0 ? "#FDE68A" : "#FFFFFF",
      starAlpha * (0.58 + seededUnit(31, index) * 0.34 + twinkle),
    );
  }
}

export function drawObservatoryStructures(args: DrawBuddyWorldBaseArgs): void {
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const hint = worldPaletteHint(args.world);
  const intensity = worldIntensity(args.world);

  if (hint === "storm") {
    fillCircle(
      args.ctx,
      width * 0.75,
      height * 0.18,
      68,
      "#818CF8",
      0.035 + intensity * 0.018,
    );
    fillCircle(args.ctx, width * 0.2, height * 0.2, 48, "#0EA5E9", 0.024);
  }

  drawSkyStructures(args);
}

export function drawCelestial(args: DrawBuddyWorldBaseArgs): void {
  const { ctx, world } = args;
  const frame = safeFrame(args.frame);
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const x = pctX(width, world.celestialX);
  const phase = worldPhase(world);
  const isNight = phase === "night";
  const isMorning = phase === "morning";
  const isEvening = phase === "evening";
  const rawY =
    pctY(height, world.celestialY) + wave(frame, 34, 0, 2, args.reducedMotion);
  const y = Math.max(isNight ? 38 : 50, rawY);
  const color = isNight
    ? "#E0E7FF"
    : isEvening
      ? "#FB923C"
      : isMorning
        ? "#FDE68A"
        : "#FBBF24";
  const glowColor = isNight ? "#818CF8" : isEvening ? "#FB7185" : "#FBBF24";

  fillCircle(ctx, x, y, isNight ? 34 : 42, glowColor, isNight ? 0.24 : 0.26);
  fillPixelRect(ctx, x - 13, y - 13, 26, 26, color);
  fillPixelRect(ctx, x - 18, y - 8, 36, 16, color);
  fillPixelRect(ctx, x - 8, y - 18, 16, 36, color);

  if (isNight) {
    fillPixelRect(ctx, x + 4, y - 13, 14, 26, "#4C1D95");
    return;
  }

  fillPixelRect(ctx, x - 2, y - 32, 4, 8, "#F59E0B");
  fillPixelRect(ctx, x - 2, y + 24, 4, 8, "#F59E0B");
  fillPixelRect(ctx, x - 32, y - 2, 8, 4, "#F59E0B");
  fillPixelRect(ctx, x + 24, y - 2, 8, 4, "#F59E0B");
  if (isMorning || isEvening) {
    fillEllipse(
      ctx,
      x,
      y + 22,
      isMorning ? 52 : 64,
      isMorning ? 8 : 10,
      isMorning ? "#FDE68A" : "#FDBA74",
      alphaForMotion(0.18, args.reducedMotion),
    );
  }
}

function drawSunMotes(args: DrawBuddyWorldBaseArgs): void {
  if (worldPhase(args.world) === "night") return;

  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const frame = safeFrame(args.frame);
  const count = countForMotion(28, args.compact, args.reducedMotion);

  for (let index = 0; index < count; index += 1) {
    const drift = args.reducedMotion
      ? 0
      : frame * (0.06 + seededUnit(41, index) * 0.08);
    const x = (seededUnit(37, index) * width + drift) % width;
    const y = height * 0.1 + seededUnit(43, index) * height * 0.48;
    const alpha = alphaForMotion(
      0.18 + seededUnit(47, index) * 0.22,
      args.reducedMotion,
    );
    drawSpark(
      args.ctx,
      x,
      y,
      1.4 + seededUnit(53, index) * 1.5,
      "#FDE68A",
      alpha,
    );
  }
}

function drawMoths(args: DrawBuddyWorldBaseArgs): void {
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const frame = safeFrame(args.frame);
  const count = countForMotion(16, args.compact, args.reducedMotion);

  for (let index = 0; index < count; index += 1) {
    const x = seededRange(59, index, width * 0.06, width * 0.94);
    const y = seededRange(61, index, height * 0.18, height * 0.58);
    const flutter = wave(frame, 24 + index, index, 5, args.reducedMotion);
    const alpha = alphaForMotion(
      0.18 + seededUnit(67, index) * 0.2,
      args.reducedMotion,
    );
    fillPixelRect(args.ctx, x - 2, y + flutter, 3, 3, "#FDE68A", alpha);
    fillPixelRect(
      args.ctx,
      x + 2,
      y + flutter + 1,
      3,
      2,
      "#C4B5FD",
      alpha * 0.72,
    );
  }
}

function drawFireflies(args: DrawBuddyWorldBaseArgs): void {
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const frame = safeFrame(args.frame);
  const count = countForMotion(24, args.compact, args.reducedMotion);

  for (let index = 0; index < count; index += 1) {
    const x = width * 0.14 + seededUnit(71, index) * width * 0.72;
    const y = height * 0.38 + seededUnit(73, index) * height * 0.34;
    const orbitX = wave(
      frame,
      34 + index,
      index * 1.7,
      10 + seededUnit(79, index) * 9,
      args.reducedMotion,
    );
    const orbitY = wave(
      frame,
      28 + index,
      index * 1.2,
      6 + seededUnit(83, index) * 5,
      args.reducedMotion,
    );
    const alpha = alphaForMotion(
      0.34 + seededUnit(89, index) * 0.42,
      args.reducedMotion,
    );
    drawSpark(
      args.ctx,
      x + orbitX,
      y + orbitY,
      1.6 + seededUnit(97, index) * 1.6,
      "#FDE68A",
      alpha,
    );
  }
}

function drawCozyHomeGlow(args: DrawBuddyWorldBaseArgs): void {
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const frame = safeFrame(args.frame);
  const x = pctX(width, BUDDY_WORLD_HOME_HOTSPOT.x);
  const y = pctY(height, BUDDY_WORLD_HOME_HOTSPOT.y);
  const intensity = worldIntensity(args.world);
  const alpha = alphaForMotion(0.18 + intensity * 0.18, args.reducedMotion);
  const count = countForMotion(9, args.compact, args.reducedMotion);

  fillCircle(
    args.ctx,
    x + 12,
    y + 6,
    args.compact ? 44 : 62,
    "#FBBF24",
    alpha * 0.34,
  );
  fillEllipse(
    args.ctx,
    x + 22,
    y + 33,
    args.compact ? 58 : 78,
    15,
    "#FDBA74",
    alpha * 0.28,
  );

  for (let index = 0; index < count; index += 1) {
    const angle = (index / count) * TAU;
    const radius = 18 + seededUnit(137, index) * (args.compact ? 30 : 42);
    const pulse = wave(frame, 40 + index, index, 3, args.reducedMotion);
    const hx = x + 23 + Math.cos(angle) * radius;
    const hy = y + 4 + Math.sin(angle) * radius * 0.46 + pulse;
    fillCircle(args.ctx, hx, hy, 4, "#F9A8D4", alpha * 0.16);
    fillPixelRect(args.ctx, hx - 2, hy - 1, 3, 3, "#F9A8D4", alpha);
    fillPixelRect(args.ctx, hx + 1, hy - 1, 3, 3, "#FCA5A5", alpha * 0.9);
    fillPixelRect(args.ctx, hx, hy + 2, 3, 3, "#FDE68A", alpha * 0.74);
  }
}

function drawAurora(args: DrawBuddyWorldBaseArgs, alpha = 0.3): void {
  const { ctx } = args;
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const y = pctY(height, args.world.weatherY);
  const frame = safeFrame(args.frame);
  const bands = countForMotion(3, args.compact, args.reducedMotion);

  for (let index = 0; index < bands; index += 1) {
    const color = index % 2 === 0 ? "#2DD4BF" : "#A855F7";
    strokeBezier(
      ctx,
      {
        x: 0,
        y: y + index * 10 + wave(frame, 90, index, 3, args.reducedMotion),
      },
      { x: width * 0.28, y: y - 28 + index * 8 },
      { x: width * 0.6, y: y + 36 - index * 6 },
      { x: width, y: y - 8 + index * 8 },
      color,
      args.compact ? 5 : 8,
      alphaForMotion(alpha, args.reducedMotion),
    );
  }
}

export function drawAmbientLayers(args: DrawBuddyWorldBaseArgs): void {
  if (hasWorldLayer(args.world, "sun_motes")) drawSunMotes(args);
  if (hasWorldLayer(args.world, "moths")) drawMoths(args);
  if (hasWorldLayer(args.world, "fireflies")) drawFireflies(args);
  if (hasWorldLayer(args.world, "aurora")) drawAurora(args, 0.28);
  if (hasWorldLayer(args.world, "cozy_home_glow")) drawCozyHomeGlow(args);
}

function drawRain(args: DrawBuddyWorldBaseArgs): void {
  const x = pctX(args.width, args.world.weatherX);
  const y = pctY(args.height, args.world.weatherY);
  const frame = safeFrame(args.frame);
  const count = countForMotion(18, args.compact, args.reducedMotion);

  drawCloud(
    args.ctx,
    x - 45,
    y - 10,
    args.compact ? 1.12 : 1.45,
    "#94A3B8",
    0.84,
  );
  for (let index = 0; index < count; index += 1) {
    const rx = x - 54 + ((index * 13 + frame) % 112);
    const ry = y + 18 + ((index * 19 + frame * 2) % 72);
    fillPixelRect(args.ctx, rx, ry, 2, 7, "#38BDF8", 0.72);
  }
}

function drawWind(args: DrawBuddyWorldBaseArgs): void {
  const x = pctX(args.width, args.world.weatherX);
  const y = pctY(args.height, args.world.weatherY);
  const frame = safeFrame(args.frame);
  const count = countForMotion(5, args.compact, args.reducedMotion);

  for (let index = 0; index < count; index += 1) {
    const speed = args.reducedMotion ? 0 : frame * (1 + index * 0.22);
    const wx = x - 70 + ((speed + index * 36) % 150);
    const wy = y + index * 12;
    fillPixelRect(args.ctx, wx, wy, 36, 2, "#FFFFFF", 0.52);
    fillPixelRect(args.ctx, wx + 28, wy + 3, 18, 2, "#FFFFFF", 0.38);
  }
}

function drawBusyCurrents(args: DrawBuddyWorldBaseArgs): void {
  const x = pctX(args.width, args.world.weatherX);
  const y = pctY(args.height, args.world.weatherY);
  const frame = safeFrame(args.frame);
  const rings = countForMotion(2, args.compact, args.reducedMotion);

  for (let index = 0; index < rings; index += 1) {
    strokeCircle(
      args.ctx,
      x,
      y,
      7 + index * 7 + wave(frame, 22, index, 1, args.reducedMotion),
      "#60A5FA",
      1,
      0.1,
    );
  }

  const workshop = objectAnchor(args, "providers", { x: 64, y: 73 });
  const beamCount = countForMotion(4, args.compact, args.reducedMotion);
  for (let index = 0; index < beamCount; index += 1) {
    const offset = (index - (beamCount - 1) / 2) * 8;
    strokeBezier(
      args.ctx,
      { x: workshop.x - 88 + index * 18, y: workshop.y + 8 },
      { x: workshop.x - 32, y: workshop.y - 32 + offset },
      { x: x - 24 + offset, y: y - 20 },
      { x, y },
      index % 2 === 0 ? "#38BDF8" : "#A78BFA",
      args.compact ? 2 : 3,
      alphaForMotion(0.1, args.reducedMotion),
    );
  }
}

function drawDreamLetters(args: DrawBuddyWorldBaseArgs): void {
  const x = pctX(args.width, args.world.weatherX);
  const y = pctY(args.height, args.world.weatherY);
  const frame = safeFrame(args.frame);
  const count = countForMotion(4, args.compact, args.reducedMotion);

  for (let index = 0; index < count; index += 1) {
    drawPixelText(
      args.ctx,
      "Z",
      x + index * 20,
      y + wave(frame, 16, index, 8, args.reducedMotion),
      "#C4B5FD",
      0.8 - index * 0.1,
    );
  }
}

function drawProviderStorm(args: DrawBuddyWorldBaseArgs): void {
  const x = pctX(args.width, args.world.weatherX);
  const y = pctY(args.height, args.world.weatherY);
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const frame = safeFrame(args.frame);
  const intensity = worldIntensity(args.world);
  const rainCount = countForMotion(18, args.compact, args.reducedMotion);
  const boltAlpha = alphaForMotion(0.68 + intensity * 0.22, args.reducedMotion);

  fillRect(args.ctx, 0, 0, width, height, "#020617", 0.12 + intensity * 0.08);
  drawCloud(
    args.ctx,
    x - 42,
    y - 15,
    args.compact ? 1.16 : 1.5,
    "#475569",
    0.94,
  );
  fillPixelRect(args.ctx, x + 4, y + 26, 8, 22, "#FACC15", boltAlpha);
  fillPixelRect(args.ctx, x - 2, y + 40, 8, 16, "#FACC15", boltAlpha);
  strokeLine(
    args.ctx,
    { x: x + 8, y: y + 28 },
    { x: x - 16, y: y + 64 },
    "#FDE68A",
    2,
    boltAlpha * 0.64,
  );

  for (let index = 0; index < rainCount; index += 1) {
    const rx = x - 60 + ((index * 17 + frame * 2) % 130);
    const ry = y + 18 + ((index * 11 + frame) % 64);
    fillPixelRect(args.ctx, rx, ry, 2, 8, "#7DD3FC", 0.72);
  }
}

function drawDreamMist(args: DrawBuddyWorldBaseArgs): void {
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const frame = safeFrame(args.frame);
  const count = countForMotion(7, args.compact, args.reducedMotion);
  const alpha = alphaForMotion(
    0.14 + worldIntensity(args.world) * 0.12,
    args.reducedMotion,
  );

  for (let index = 0; index < count; index += 1) {
    const y = height * 0.28 + index * height * 0.055;
    const x = ((frame * 0.2 + index * 97) % (width + 160)) - 80;
    fillEllipse(
      args.ctx,
      x,
      y + wave(frame, 42, index, 5, args.reducedMotion),
      args.compact ? 46 : 68,
      args.compact ? 8 : 12,
      "#C4B5FD",
      alpha * (0.65 + seededUnit(101, index) * 0.3),
    );
  }

  if (worldPaletteHint(args.world) === "dream") {
    fillRect(
      args.ctx,
      0,
      0,
      width,
      height,
      "#C4B5FD",
      alphaForMotion(0.035, args.reducedMotion),
    );
  }
}

function drawProviderFlicker(args: DrawBuddyWorldBaseArgs): void {
  const anchor = objectAnchor(args, "providers", { x: 72, y: 67 });
  const frame = safeFrame(args.frame);
  const count = countForMotion(6, args.compact, args.reducedMotion);
  const alpha = alphaForMotion(
    0.12 + worldIntensity(args.world) * 0.14,
    args.reducedMotion,
  );

  for (let index = 0; index < count; index += 1) {
    const angle = (index / count) * TAU;
    const flicker = args.reducedMotion ? 0 : Math.sin(frame / 10 + index) * 4;
    const radius = 24 + seededUnit(103, index) * 22 + flicker;
    drawSpark(
      args.ctx,
      anchor.x + Math.cos(angle) * radius,
      anchor.y - 42 + Math.sin(angle) * radius * 0.34,
      2 + seededUnit(107, index) * 2,
      index % 2 === 0 ? "#FDE68A" : "#60A5FA",
      alpha,
    );
  }

  strokeCircle(
    args.ctx,
    anchor.x,
    anchor.y - 22,
    28 + wave(frame, 34, 0, 2, args.reducedMotion),
    "#F59E0B",
    1.5,
    alpha * 0.42,
  );
}

function drawWorkshopRunes(args: DrawBuddyWorldBaseArgs): void {
  const anchor = objectAnchor(args, "providers", { x: 64, y: 73 });
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const frame = safeFrame(args.frame);
  const count = countForMotion(10, args.compact, args.reducedMotion);
  const alpha = alphaForMotion(
    0.18 + worldIntensity(args.world) * 0.22,
    args.reducedMotion,
  );
  const center = {
    x: clamp(anchor.x - width * 0.18, width * 0.25, width * 0.7),
    y: height * 0.72,
  };

  fillCircle(
    args.ctx,
    center.x + 18,
    center.y - 14,
    args.compact ? 32 : 44,
    "#2563EB",
    alpha * 0.08,
  );
  fillEllipse(
    args.ctx,
    center.x + 22,
    center.y + 6,
    args.compact ? 42 : 58,
    10,
    "#38BDF8",
    alpha * 0.1,
  );

  for (let index = 0; index < count; index += 1) {
    const t = count === 1 ? 0 : index / (count - 1);
    const x = lerp(width * 0.42, center.x + 52, t);
    const y =
      center.y +
      Math.sin(t * Math.PI * 2 + frame / 28) * (args.reducedMotion ? 0 : 8);
    strokeLine(
      args.ctx,
      { x: x - 7, y },
      { x: x + 7, y },
      index % 2 === 0 ? "#60A5FA" : "#A78BFA",
      2,
      alpha * (0.72 + seededUnit(109, index) * 0.24),
    );
    strokeLine(
      args.ctx,
      { x, y: y - 7 },
      { x, y: y + 7 },
      "#FDE68A",
      1.4,
      alpha * 0.6,
    );
  }

  const sparkCount = countForMotion(8, args.compact, args.reducedMotion);
  for (let index = 0; index < sparkCount; index += 1) {
    const x = center.x - 28 + seededUnit(139, index) * 116;
    const y = center.y - 38 + seededUnit(149, index) * 54;
    drawSpark(
      args.ctx,
      x + wave(frame, 20 + index, index, 5, args.reducedMotion),
      y,
      1.5 + seededUnit(151, index) * 1.2,
      index % 2 === 0 ? "#67E8F9" : "#FDE68A",
      alpha * 0.9,
    );
  }
}

function drawMemoryOrbs(args: DrawBuddyWorldBaseArgs): void {
  const anchor = objectAnchor(args, "memory", { x: 33, y: 52 });
  const workshop = objectAnchor(args, "providers", { x: 64, y: 73 });
  const frame = safeFrame(args.frame);
  const count = countForMotion(9, args.compact, args.reducedMotion);
  const memoryObject = worldObjects(args.world).find(
    (object) => object.id === "memory",
  );
  const streaming = memoryObject?.animation === "stream";
  const critical = memoryObject?.state === "critical";
  const alpha = alphaForMotion(
    0.22 + worldIntensity(args.world) * (critical ? 0.36 : 0.28),
    args.reducedMotion,
  );

  for (let index = 0; index < count; index += 1) {
    const angle =
      (index / count) * TAU + wave(frame, 88, index, 0.7, args.reducedMotion);
    const radius = 18 + seededUnit(113, index) * (args.compact ? 34 : 48);
    const orbitX = anchor.x + Math.cos(angle) * radius;
    const orbitY = anchor.y - 16 + Math.sin(angle) * radius * 0.46;
    const streamProgress = streaming
      ? (seededUnit(157, index) + (args.reducedMotion ? 0 : frame / 160)) % 1
      : 0;
    const x = streaming
      ? lerp(orbitX, workshop.x - 48, streamProgress)
      : orbitX;
    const y = streaming
      ? lerp(orbitY, workshop.y - 42, streamProgress) -
        Math.sin(streamProgress * Math.PI) * 18
      : orbitY;
    const color = critical
      ? index % 3 === 0
        ? "#EF4444"
        : "#F59E0B"
      : index % 3 === 0
        ? "#FEF3C7"
        : "#FBBF24";
    fillCircle(
      args.ctx,
      x,
      y,
      4 + seededUnit(127, index) * 4,
      color,
      alpha * (critical ? 0.12 : 0.08),
    );
    drawSpark(args.ctx, x, y, 1.6 + seededUnit(131, index) * 1.8, color, alpha);
  }

  if (streaming) {
    strokeBezier(
      args.ctx,
      { x: anchor.x + 18, y: anchor.y - 20 },
      { x: anchor.x + 88, y: anchor.y - 58 },
      { x: workshop.x - 86, y: workshop.y - 64 },
      { x: workshop.x - 40, y: workshop.y - 36 },
      "#FDE68A",
      args.compact ? 2 : 3,
      alpha * 0.26,
    );
  }
}

function drawToyGlow(args: DrawBuddyWorldBaseArgs): void {
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const x = width * 0.52;
  const y = height * 0.83;
  const alpha = alphaForMotion(
    0.18 + worldIntensity(args.world) * 0.14,
    args.reducedMotion,
  );
  fillEllipse(args.ctx, x, y, args.compact ? 30 : 44, 8, "#F9A8D4", alpha);
  strokeCircle(
    args.ctx,
    x + 4,
    y - 10,
    args.compact ? 15 : 21,
    "#F9A8D4",
    2,
    alpha * 0.7,
  );
  fillPixelRect(args.ctx, x - 14, y - 20, 13, 7, "#A78BFA", alpha * 0.86);
  fillPixelRect(args.ctx, x - 1, y - 24, 15, 7, "#60A5FA", alpha * 0.86);
  drawSpark(args.ctx, x + 18, y - 8, 2.5, "#FDE68A", alpha * 1.3);
}

function drawEmptyFoodNook(args: DrawBuddyWorldBaseArgs): void {
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const x = width * 0.19;
  const y = height * 0.82;
  const alpha = alphaForMotion(
    0.28 + worldIntensity(args.world) * 0.14,
    args.reducedMotion,
  );
  fillEllipse(args.ctx, x, y + 2, 18, 5, "#92400E", alpha);
  fillEllipse(args.ctx, x, y, 13, 4, "#FDE68A", alpha * 0.55);
  strokeEllipse(args.ctx, x, y - 1, 18, 6, "#FDE68A", 2, alpha * 0.74);
  drawSpark(args.ctx, x + 23, y - 14, 2.2, "#FDE68A", alpha * 0.86);
}

export function drawWeatherAtmosphere(args: DrawBuddyWorldBaseArgs): void {
  const weather = worldWeather(args.world);

  if (weather === "rain") drawRain(args);
  if (weather === "wind") drawWind(args);
  if (weather === "busy") drawBusyCurrents(args);
  if (weather === "dream") drawDreamLetters(args);
  if (weather === "storm" || hasWorldLayer(args.world, "provider_storm")) {
    drawProviderStorm(args);
  }

  if (hasWorldLayer(args.world, "dream_mist")) drawDreamMist(args);
  if (hasWorldLayer(args.world, "provider_flicker")) drawProviderFlicker(args);
  if (hasWorldLayer(args.world, "workshop_runes")) drawWorkshopRunes(args);
  if (hasWorldLayer(args.world, "memory_orbs")) drawMemoryOrbs(args);
  if (hasWorldLayer(args.world, "toy_glow")) drawToyGlow(args);
  if (hasWorldLayer(args.world, "empty_food_nook")) drawEmptyFoodNook(args);
}
