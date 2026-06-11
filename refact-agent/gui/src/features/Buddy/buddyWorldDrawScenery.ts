import {
  alphaForMotion,
  countForMotion,
  fillCircle,
  fillEllipse,
  fillPixelRect,
  pctX,
  pctY,
  safeDimension,
  safeFrame,
  seededUnit,
  strokeBezier,
  strokeCircle,
  strokeLine,
  wave,
  worldPaletteHint,
  worldPhase,
  worldWeather,
  type DrawBuddyWorldBaseArgs,
} from "./buddyWorldDrawHelpers";

interface PhaseTints {
  ridgeFar: string;
  ridgeNear: string;
  snow: string;
  cloudTop: string;
  cloudBase: string;
  canopyDeep: string;
  canopyMid: string;
  canopyLight: string;
  trunkDark: string;
  trunkLight: string;
}

function phaseTints(args: DrawBuddyWorldBaseArgs): PhaseTints {
  const hint = worldPaletteHint(args.world);
  if (hint === "night" || hint === "dream") {
    return {
      ridgeFar: "#2A3A60",
      ridgeNear: "#22304F",
      snow: "#9FB2D8",
      cloudTop: "#36456E",
      cloudBase: "#222F52",
      canopyDeep: "#16301F",
      canopyMid: "#1F4129",
      canopyLight: "#2C5638",
      trunkDark: "#241A14",
      trunkLight: "#3A2C20",
    };
  }
  if (hint === "storm") {
    return {
      ridgeFar: "#55657A",
      ridgeNear: "#46566B",
      snow: "#C2CEDC",
      cloudTop: "#7E8DA0",
      cloudBase: "#5A6A7E",
      canopyDeep: "#23402C",
      canopyMid: "#33543A",
      canopyLight: "#48704C",
      trunkDark: "#33261C",
      trunkLight: "#4A382A",
    };
  }
  if (hint === "dusk") {
    return {
      ridgeFar: "#7B6A93",
      ridgeNear: "#5E5680",
      snow: "#F2D4C0",
      cloudTop: "#F4CDB0",
      cloudBase: "#C98D96",
      canopyDeep: "#2C4A30",
      canopyMid: "#3F6340",
      canopyLight: "#5E8254",
      trunkDark: "#3D2B20",
      trunkLight: "#5C4332",
    };
  }
  if (hint === "dawn") {
    return {
      ridgeFar: "#9BAFD0",
      ridgeNear: "#7E96BC",
      snow: "#FBEEDC",
      cloudTop: "#FAEAD3",
      cloudBase: "#E2B391",
      canopyDeep: "#33583A",
      canopyMid: "#4A7449",
      canopyLight: "#6D965F",
      trunkDark: "#4A362A",
      trunkLight: "#69503C",
    };
  }
  return {
    ridgeFar: "#93AECB",
    ridgeNear: "#7E9DBF",
    snow: "#F4F8FB",
    cloudTop: "#FBFDFE",
    cloudBase: "#BCD4E6",
    canopyDeep: "#2F6B3F",
    canopyMid: "#4F8F54",
    canopyLight: "#74B06A",
    trunkDark: "#4A362A",
    trunkLight: "#6B4F3A",
  };
}

export function drawAlpineRidge(args: DrawBuddyWorldBaseArgs): void {
  const { ctx } = args;
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const tints = phaseTints(args);
  const baseY = height * 0.66;

  ctx.save();
  ctx.globalAlpha = alphaForMotion(0.5, args.reducedMotion);
  ctx.fillStyle = tints.ridgeFar;
  ctx.beginPath();
  ctx.moveTo(0, baseY);
  const peaks = [
    { x: 0.06, y: 0.5 },
    { x: 0.16, y: 0.56 },
    { x: 0.27, y: 0.46 },
    { x: 0.38, y: 0.55 },
    { x: 0.52, y: 0.44 },
    { x: 0.63, y: 0.54 },
    { x: 0.76, y: 0.47 },
    { x: 0.88, y: 0.56 },
    { x: 0.97, y: 0.5 },
  ];
  for (const peak of peaks) {
    ctx.lineTo(width * peak.x, height * peak.y);
  }
  ctx.lineTo(width, baseY);
  ctx.closePath();
  ctx.fill();
  ctx.restore();

  for (const peak of [peaks[2], peaks[4], peaks[6]]) {
    const px = width * peak.x;
    const py = height * peak.y;
    fillPixelRect(args.ctx, px - 3, py, 6, 2, tints.snow, 0.55);
    fillPixelRect(args.ctx, px - 1, py - 1, 3, 1, tints.snow, 0.62);
    fillPixelRect(args.ctx, px - 5, py + 2, 4, 1, tints.snow, 0.4);
    fillPixelRect(args.ctx, px + 2, py + 2, 4, 1, tints.snow, 0.34);
  }

  ctx.save();
  ctx.globalAlpha = alphaForMotion(0.6, args.reducedMotion);
  ctx.fillStyle = tints.ridgeNear;
  ctx.beginPath();
  ctx.moveTo(0, baseY + 6);
  for (let x = 0; x <= width; x += 24) {
    const t = x / width;
    const y =
      height *
      (0.56 + Math.sin(t * 9.2 + 1.7) * 0.045 + Math.sin(t * 3.1) * 0.03);
    ctx.lineTo(x, y);
  }
  ctx.lineTo(width, baseY + 6);
  ctx.closePath();
  ctx.fill();
  ctx.restore();
}

function drawCumulus(
  args: DrawBuddyWorldBaseArgs,
  x: number,
  y: number,
  scale: number,
  alpha: number,
): void {
  const tints = phaseTints(args);
  const puffs = [
    { dx: 0, dy: 6, r: 9 },
    { dx: 10, dy: 0, r: 12 },
    { dx: 24, dy: -4, r: 14 },
    { dx: 38, dy: 2, r: 11 },
    { dx: 48, dy: 7, r: 8 },
    { dx: 18, dy: 8, r: 10 },
    { dx: 32, dy: 9, r: 9 },
  ];
  fillEllipse(
    args.ctx,
    x + 24 * scale,
    y + 14 * scale,
    30 * scale,
    6 * scale,
    tints.cloudBase,
    alpha * 0.9,
  );
  for (const puff of puffs) {
    fillCircle(
      args.ctx,
      x + puff.dx * scale,
      y + puff.dy * scale,
      puff.r * scale,
      tints.cloudTop,
      alpha,
    );
  }
  fillCircle(
    args.ctx,
    x + 16 * scale,
    y + 10 * scale,
    9 * scale,
    tints.cloudBase,
    alpha * 0.5,
  );
  fillCircle(
    args.ctx,
    x + 34 * scale,
    y + 11 * scale,
    8 * scale,
    tints.cloudBase,
    alpha * 0.45,
  );
}

export function drawGhibliClouds(args: DrawBuddyWorldBaseArgs): void {
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const frame = safeFrame(args.frame);
  const hint = worldPaletteHint(args.world);
  if (hint === "storm") return;
  const count = countForMotion(3, args.compact, args.reducedMotion);
  const baseAlpha = hint === "night" || hint === "dream" ? 0.5 : 0.85;

  for (let index = 0; index < count; index += 1) {
    const speed = 0.06 + seededUnit(311, index) * 0.05;
    const drift = args.reducedMotion ? index * 130 : frame * speed;
    const x = ((seededUnit(313, index) * width + drift) % (width + 260)) - 130;
    const y = height * (0.06 + seededUnit(317, index) * 0.2);
    const scale = (args.compact ? 0.6 : 0.85) + seededUnit(331, index) * 0.55;
    drawCumulus(
      args,
      x,
      y,
      scale,
      baseAlpha * (0.7 + seededUnit(337, index) * 0.3),
    );
  }
}

export function drawGreatTree(args: DrawBuddyWorldBaseArgs): void {
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const frame = safeFrame(args.frame);
  const tints = phaseTints(args);
  const baseX = width * 0.3;
  const baseY = height * 0.72;
  const sway = wave(frame, 90, 0, 3, args.reducedMotion);
  const swayHigh = wave(frame, 90, 0.9, 5, args.reducedMotion);
  const scale = args.compact ? 0.72 : 1;

  fillEllipse(
    args.ctx,
    baseX,
    baseY + 4,
    34 * scale,
    7 * scale,
    "#1A2E20",
    0.3,
  );

  fillPixelRect(
    args.ctx,
    baseX - 7 * scale,
    baseY - 38 * scale,
    7 * scale,
    42 * scale,
    tints.trunkDark,
    0.96,
  );
  fillPixelRect(
    args.ctx,
    baseX,
    baseY - 38 * scale,
    6 * scale,
    42 * scale,
    tints.trunkLight,
    0.96,
  );
  fillPixelRect(
    args.ctx,
    baseX - 13 * scale,
    baseY - 2,
    8 * scale,
    6 * scale,
    tints.trunkDark,
    0.9,
  );
  fillPixelRect(
    args.ctx,
    baseX + 5 * scale,
    baseY - 2,
    9 * scale,
    6 * scale,
    tints.trunkLight,
    0.88,
  );
  fillPixelRect(
    args.ctx,
    baseX - 5 * scale,
    baseY - 50 * scale,
    4 * scale,
    16 * scale,
    tints.trunkDark,
    0.92,
  );
  fillPixelRect(
    args.ctx,
    baseX + 3 * scale,
    baseY - 48 * scale,
    4 * scale,
    14 * scale,
    tints.trunkLight,
    0.9,
  );

  const canopy = [
    { dx: -34, dy: -52, rx: 26, ry: 14, color: tints.canopyDeep, swayK: 0.5 },
    { dx: 26, dy: -56, rx: 30, ry: 15, color: tints.canopyDeep, swayK: 0.6 },
    { dx: -12, dy: -68, rx: 32, ry: 16, color: tints.canopyMid, swayK: 0.8 },
    { dx: 18, dy: -74, rx: 26, ry: 13, color: tints.canopyMid, swayK: 0.9 },
    { dx: -2, dy: -84, rx: 24, ry: 12, color: tints.canopyLight, swayK: 1 },
    { dx: -28, dy: -64, rx: 18, ry: 9, color: tints.canopyMid, swayK: 0.7 },
  ];
  for (const blob of canopy) {
    fillEllipse(
      args.ctx,
      baseX +
        blob.dx * scale +
        (blob.swayK >= 0.8 ? swayHigh : sway) * blob.swayK,
      baseY + blob.dy * scale,
      blob.rx * scale,
      blob.ry * scale,
      blob.color,
      0.94,
    );
  }

  const fleckCount = countForMotion(7, args.compact, args.reducedMotion);
  for (let index = 0; index < fleckCount; index += 1) {
    const fx =
      baseX +
      (seededUnit(401, index) - 0.5) * 60 * scale +
      wave(frame, 30 + index, index, 2, args.reducedMotion);
    const fy = baseY - (52 + seededUnit(409, index) * 36) * scale;
    fillPixelRect(
      args.ctx,
      fx,
      fy,
      2,
      2,
      tints.canopyLight,
      0.5 + seededUnit(419, index) * 0.3,
    );
  }
}

export function drawSkyIsland(args: DrawBuddyWorldBaseArgs): void {
  if (worldPhase(args.world) !== "day") return;
  if (worldWeather(args.world) !== "clear") return;
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const frame = safeFrame(args.frame);
  const cycle = 5200;
  const within = args.reducedMotion ? 420 : (frame * 0.9) % cycle;
  if (within > 1400) return;
  const t = within / 1400;
  const x = width * (0.92 - t * 0.5);
  const y = height * 0.14 + wave(frame, 120, 0, 3, args.reducedMotion);
  const alpha = alphaForMotion(
    0.22 * Math.min(1, Math.min(t * 6, (1 - t) * 6)),
    args.reducedMotion,
  );
  const s = args.compact ? 0.6 : 0.85;

  fillEllipse(args.ctx, x, y, 22 * s, 6 * s, "#5E7E93", alpha);
  fillPixelRect(
    args.ctx,
    x - 10 * s,
    y + 4 * s,
    20 * s,
    5 * s,
    "#54707F",
    alpha,
  );
  fillPixelRect(
    args.ctx,
    x - 5 * s,
    y + 9 * s,
    10 * s,
    4 * s,
    "#4A6271",
    alpha,
  );
  fillPixelRect(args.ctx, x - 1, y + 13 * s, 3, 3 * s, "#42586B", alpha * 0.8);
  fillEllipse(args.ctx, x - 4 * s, y - 5 * s, 9 * s, 4 * s, "#6E9272", alpha);
  fillEllipse(args.ctx, x + 7 * s, y - 4 * s, 6 * s, 3 * s, "#7FA37F", alpha);
  fillPixelRect(args.ctx, x - 1, y - 10 * s, 2, 4 * s, "#8FA9B8", alpha);
  strokeLine(
    args.ctx,
    { x: x + 3 * s, y: y + 5 * s },
    { x: x + 3 * s, y: y + 16 * s },
    "#BFD9E8",
    1,
    alpha * 0.7,
  );
  fillEllipse(
    args.ctx,
    x + 3 * s,
    y + 17 * s,
    4 * s,
    1.6 * s,
    "#BFD9E8",
    alpha * 0.4,
  );
}

export function drawAirship(args: DrawBuddyWorldBaseArgs): void {
  const phase = worldPhase(args.world);
  if (phase !== "morning" && phase !== "evening") return;
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const frame = safeFrame(args.frame);
  const cycle = 4600;
  const within = args.reducedMotion ? 700 : (frame * 1.1) % cycle;
  if (within > 1600) return;
  const t = within / 1600;
  const x = width * (-0.08 + t * 1.16);
  const y = height * 0.18 + wave(frame, 70, 0, 2, args.reducedMotion);
  const alpha = alphaForMotion(
    0.5 * Math.min(1, Math.min(t * 8, (1 - t) * 8)),
    args.reducedMotion,
  );
  const s = args.compact ? 0.7 : 1;

  fillEllipse(args.ctx, x, y, 14 * s, 5 * s, "#8A7A6D", alpha);
  fillEllipse(
    args.ctx,
    x - 2 * s,
    y - 1.6 * s,
    11 * s,
    2.6 * s,
    "#A49483",
    alpha * 0.8,
  );
  fillPixelRect(
    args.ctx,
    x - 12 * s,
    y - 4 * s,
    4 * s,
    4 * s,
    "#6E6157",
    alpha,
  );
  fillPixelRect(args.ctx, x - 3 * s, y + 5 * s, 7 * s, 3 * s, "#5C5249", alpha);
  fillPixelRect(
    args.ctx,
    x + 10 * s,
    y - 5 * s,
    3 * s,
    10 * s,
    "#6E6157",
    alpha * 0.9,
  );
  const prop = Math.floor(frame / 4) % 2 === 0;
  fillPixelRect(
    args.ctx,
    x - 16 * s,
    y - (prop ? 3 : 0),
    2,
    prop ? 6 : 2,
    "#4A423B",
    alpha,
  );
  if (phase === "evening") {
    fillPixelRect(
      args.ctx,
      x - 1,
      y + 5.6 * s,
      1.6,
      1.6,
      "#FDE68A",
      alpha * 1.4,
    );
    fillPixelRect(
      args.ctx,
      x + 3 * s,
      y + 5.6 * s,
      1.6,
      1.6,
      "#FDE68A",
      alpha * 1.2,
    );
  }
}

export function drawKomorebi(args: DrawBuddyWorldBaseArgs): void {
  const phase = worldPhase(args.world);
  if (phase !== "morning" && phase !== "day") return;
  const weather = worldWeather(args.world);
  if (weather !== "clear" && weather !== "wind") return;
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const frame = safeFrame(args.frame);
  const sunX = pctX(width, args.world.celestialX);
  const sunY = pctY(height, args.world.celestialY);
  const beams = countForMotion(3, args.compact, args.reducedMotion);

  for (let index = 0; index < beams; index += 1) {
    const spread = (index - (beams - 1) / 2) * 0.16;
    const sweep = wave(frame, 200, index * 1.3, 0.04, args.reducedMotion);
    const tx = width * (0.32 + spread + sweep);
    const ty = height * 0.86;
    const breathe = alphaForMotion(
      0.045 + Math.abs(wave(frame, 110, index, 0.02, args.reducedMotion)),
      args.reducedMotion,
    );
    strokeLine(
      args.ctx,
      { x: sunX + spread * 60, y: sunY + 8 },
      { x: tx, y: ty },
      "#FFF3C9",
      args.compact ? 9 : 14,
      breathe,
    );
  }
}

export function drawNightSkyDust(args: DrawBuddyWorldBaseArgs): void {
  const hint = worldPaletteHint(args.world);
  if (hint !== "night" && hint !== "dream") return;
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const frame = safeFrame(args.frame);

  const bandCount = countForMotion(12, args.compact, args.reducedMotion);
  for (let index = 0; index < bandCount; index += 1) {
    const t = index / Math.max(1, bandCount - 1);
    const x = width * (0.12 + t * 0.74) + seededUnit(503, index) * 26 - 13;
    const y = height * (0.4 - t * 0.3) + seededUnit(509, index) * 20 - 10;
    fillCircle(
      args.ctx,
      x,
      y,
      9 + seededUnit(521, index) * 13,
      "#C7D2FE",
      alphaForMotion(0.028 + seededUnit(523, index) * 0.02, args.reducedMotion),
    );
  }

  const dustCount = countForMotion(30, args.compact, args.reducedMotion);
  for (let index = 0; index < dustCount; index += 1) {
    const x = seededUnit(541, index) * width;
    const y = seededUnit(547, index) * height * 0.5;
    const twinkle = args.reducedMotion
      ? 0
      : Math.sin(frame / 36 + index * 1.7) * 0.14;
    fillPixelRect(
      args.ctx,
      x,
      y,
      1.4,
      1.4,
      index % 3 === 0 ? "#9DAFD6" : "#CBD5E1",
      0.24 + seededUnit(557, index) * 0.3 + twinkle,
    );
  }
}

export function drawSootSprites(args: DrawBuddyWorldBaseArgs): void {
  const phase = worldPhase(args.world);
  const hint = worldPaletteHint(args.world);
  if (phase !== "evening" && phase !== "night" && hint !== "dream") return;
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const frame = safeFrame(args.frame);
  const count = countForMotion(3, args.compact, args.reducedMotion);

  for (let index = 0; index < count; index += 1) {
    const homeX = width * (0.13 + seededUnit(601, index) * 0.1);
    const roamSpan = 26 + seededUnit(607, index) * 30;
    const cycle = 360 + seededUnit(613, index) * 140;
    const t = args.reducedMotion
      ? 0.5
      : ((frame + index * 167) % cycle) / cycle;
    const dartT = t < 0.5 ? t * 2 : (1 - t) * 2;
    const eased = dartT * dartT * (3 - 2 * dartT);
    const x = homeX + eased * roamSpan;
    const hop = Math.abs(
      wave(frame, 5 + index, index * 2.3, 1.6, args.reducedMotion),
    );
    const y = height * 0.875 - hop;

    fillCircle(args.ctx, x, y, 3, "#1C1917", 0.95);
    for (let spike = 0; spike < 8; spike += 1) {
      const a =
        (spike / 8) * Math.PI * 2 + (args.reducedMotion ? 0 : frame * 0.05);
      fillPixelRect(
        args.ctx,
        x + Math.cos(a) * 3.6,
        y + Math.sin(a) * 3.6,
        1,
        1,
        "#1C1917",
        0.85,
      );
    }
    const look = Math.sin(frame / 40 + index * 2) > 0 ? 0.8 : -0.8;
    fillPixelRect(args.ctx, x - 1.6 + look, y - 0.8, 1.2, 1.2, "#F8FAFC", 0.95);
    fillPixelRect(args.ctx, x + 0.8 + look, y - 0.8, 1.2, 1.2, "#F8FAFC", 0.95);
    if (index % 2 === 0) {
      const starBob = wave(frame, 14, index, 1, args.reducedMotion);
      fillPixelRect(
        args.ctx,
        x - 0.8,
        y - 6.4 + starBob,
        1.6,
        1.6,
        "#FDE68A",
        0.9,
      );
    }
  }
}

export function drawKodama(args: DrawBuddyWorldBaseArgs): void {
  if (worldPaletteHint(args.world) !== "night") return;
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const frame = safeFrame(args.frame);
  const count = countForMotion(2, args.compact, args.reducedMotion);

  for (let index = 0; index < count; index += 1) {
    const x = width * (0.24 + index * 0.09) + seededUnit(701, index) * 14;
    const y = height * (0.66 + seededUnit(709, index) * 0.04);
    const tiltTick = Math.floor(frame / (90 + index * 40)) % 3;
    const headTilt = tiltTick === 1 ? 1 : tiltTick === 2 ? -1 : 0;
    const alpha = alphaForMotion(
      0.55 + Math.abs(wave(frame, 60, index, 0.18, args.reducedMotion)),
      args.reducedMotion,
    );

    fillCircle(args.ctx, x, y - 5, 5.5, "#E8EEF4", alpha * 0.16);
    fillPixelRect(args.ctx, x - 1.4, y - 2, 3, 5, "#E8EEF4", alpha);
    fillPixelRect(
      args.ctx,
      x - 2.2 + headTilt,
      y - 6,
      4.4,
      4.4,
      "#F1F5F9",
      alpha,
    );
    fillPixelRect(
      args.ctx,
      x - 1.2 + headTilt,
      y - 5,
      1,
      1.6,
      "#3A4654",
      alpha,
    );
    fillPixelRect(
      args.ctx,
      x + 0.6 + headTilt,
      y - 5,
      1,
      1.6,
      "#3A4654",
      alpha,
    );
    fillPixelRect(
      args.ctx,
      x - 0.4 + headTilt,
      y - 3.2,
      1,
      1,
      "#3A4654",
      alpha * 0.9,
    );
  }
}

export function drawMeadowCritters(args: DrawBuddyWorldBaseArgs): void {
  const phase = worldPhase(args.world);
  const weather = worldWeather(args.world);
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const frame = safeFrame(args.frame);
  const season = args.world.season;
  const daylight =
    phase === "morning" || phase === "day" || phase === "evening";
  const calm = weather === "clear" || weather === "wind" || weather === "busy";

  if (daylight && calm) {
    const cycle = 300;
    const t = args.reducedMotion ? 0.2 : (frame % cycle) / cycle;
    const hopT = Math.min(1, Math.max(0, (t - 0.3) / 0.4));
    const bx = width * (0.58 + hopT * 0.06);
    const hopY = Math.sin(hopT * Math.PI * 3) * 4;
    const by = height * 0.885 - Math.abs(hopY);
    const fur = season === "winter" ? "#F4F1EC" : "#EDE3D3";
    const nibble = t < 0.3 && Math.floor(frame / 14) % 2 === 0 ? 0.6 : 0;

    fillEllipse(args.ctx, bx, by + 3.4, 4.4, 1.4, "#1A2E20", 0.22);
    fillEllipse(args.ctx, bx, by, 4, 3, fur, 0.96);
    fillCircle(args.ctx, bx + 3.4, by - 1.8 + nibble, 2.2, fur, 0.96);
    fillPixelRect(args.ctx, bx + 3, by - 5.4, 1.2, 3.4, fur, 0.94);
    fillPixelRect(args.ctx, bx + 4.6, by - 5.2, 1.2, 3.2, fur, 0.9);
    fillPixelRect(args.ctx, bx + 3.2, by - 4.6, 0.8, 2, "#E5B8C0", 0.7);
    fillPixelRect(args.ctx, bx + 4.4, by - 2.2, 1, 1, "#2D2A26", 0.95);
    fillCircle(args.ctx, bx - 3.6, by - 0.6, 1.5, "#F8F6F1", 0.9);
  }

  if (
    (phase === "day" || phase === "evening") &&
    season !== "winter" &&
    weather !== "rain" &&
    weather !== "storm"
  ) {
    const fx = width * 0.168;
    const fy = height * 0.862;
    const hopCycle = 260;
    const ht = args.reducedMotion ? 0 : ((frame + 60) % hopCycle) / hopCycle;
    const leap = ht > 0.82 ? Math.sin(((ht - 0.82) / 0.18) * Math.PI) : 0;
    const blink = Math.floor(frame / 50) % 7 === 0;

    fillEllipse(args.ctx, fx, fy - leap * 7, 3.4, 2.2, "#6FA557", 0.95);
    fillCircle(args.ctx, fx - 2.2, fy - 2 - leap * 7, 1.4, "#6FA557", 0.95);
    fillCircle(args.ctx, fx + 2.2, fy - 2 - leap * 7, 1.4, "#6FA557", 0.95);
    if (!blink) {
      fillPixelRect(
        args.ctx,
        fx - 2.6,
        fy - 2.6 - leap * 7,
        1,
        1,
        "#22301C",
        0.95,
      );
      fillPixelRect(
        args.ctx,
        fx + 1.8,
        fy - 2.6 - leap * 7,
        1,
        1,
        "#22301C",
        0.95,
      );
    }
    fillPixelRect(
      args.ctx,
      fx - 1.6,
      fy + 1.2 - leap * 7,
      3.4,
      1,
      "#8FBF74",
      0.85,
    );
  }

  if (phase === "morning" || phase === "day") {
    const span = width * 0.2;
    const crawl = args.reducedMotion
      ? 0.4
      : ((frame * 0.12) % (span * 2)) / span;
    const forward = crawl <= 1 ? crawl : 2 - crawl;
    const sx = width * 0.34 + forward * span;
    const sy = height * 0.935;
    const stretch = Math.abs(wave(frame, 22, 0, 0.6, args.reducedMotion));

    for (let trail = 1; trail <= 3; trail += 1) {
      fillPixelRect(
        args.ctx,
        sx - trail * 7 * (crawl <= 1 ? 1 : -1),
        sy + 1.4,
        2.4,
        1,
        "#C9DCE4",
        0.16 - trail * 0.04,
      );
    }
    fillEllipse(args.ctx, sx, sy, 3.4 + stretch, 1.6, "#B5CC8E", 0.92);
    fillCircle(args.ctx, sx - 0.6, sy - 2.2, 2.6, "#C98A5B", 0.95);
    strokeCircle(args.ctx, sx - 0.6, sy - 2.2, 1.4, "#9E663D", 1, 0.6);
    fillPixelRect(
      args.ctx,
      sx + 2.6 + stretch,
      sy - 2.6,
      0.8,
      2,
      "#8FA56F",
      0.85,
    );
    fillPixelRect(
      args.ctx,
      sx + 3.8 + stretch,
      sy - 2.8,
      0.8,
      2.2,
      "#8FA56F",
      0.85,
    );
  }

  if ((season === "spring" || season === "summer") && phase === "day" && calm) {
    const beeCount = countForMotion(3, args.compact, args.reducedMotion);
    const hiveX = width * 0.32;
    const hiveY = height * 0.78;
    for (let index = 0; index < beeCount; index += 1) {
      const orbit = 8 + seededUnit(801, index) * 12;
      const a =
        (args.reducedMotion ? index * 2.1 : frame / (16 + index * 5)) +
        index * 2.2;
      const bx = hiveX + Math.cos(a) * orbit;
      const by = hiveY + Math.sin(a * 1.4) * orbit * 0.45;
      fillPixelRect(args.ctx, bx, by, 2.4, 1.6, "#E8C44C", 0.95);
      fillPixelRect(args.ctx, bx + 0.8, by, 0.8, 1.6, "#3A3328", 0.9);
      const wingUp = Math.floor(frame / 2 + index) % 2 === 0;
      fillPixelRect(
        args.ctx,
        bx + 0.4,
        by - (wingUp ? 1.4 : 0.8),
        1.4,
        1,
        "#F1F5F9",
        0.8,
      );
    }
  }
}

export function drawStream(args: DrawBuddyWorldBaseArgs): void {
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const frame = safeFrame(args.frame);
  const frozen = args.world.season === "winter";
  const pondX = width * 0.13;
  const pondY = height * 0.875;
  const startX = -width * 0.01;
  const startY = height * 0.77;

  args.ctx.save();
  args.ctx.globalAlpha = alphaForMotion(
    frozen ? 0.5 : 0.62,
    args.reducedMotion,
  );
  args.ctx.strokeStyle = frozen ? "#BFDCEC" : "#6FA8CC";
  args.ctx.lineWidth = args.compact ? 4 : 6;
  args.ctx.lineCap = "round";
  args.ctx.beginPath();
  args.ctx.moveTo(startX, startY);
  args.ctx.bezierCurveTo(
    width * 0.05,
    height * 0.8,
    width * 0.03,
    height * 0.84,
    pondX - 14,
    pondY - 4,
  );
  args.ctx.stroke();
  args.ctx.restore();

  if (frozen || args.reducedMotion) return;
  const ticks = countForMotion(4, args.compact, false);
  for (let index = 0; index < ticks; index += 1) {
    const t = (((frame * 0.012 + index / ticks) % 1) + 1) % 1;
    const mt = 1 - t;
    const x =
      mt * mt * mt * startX +
      3 * mt * mt * t * width * 0.05 +
      3 * mt * t * t * width * 0.03 +
      t * t * t * (pondX - 14);
    const y =
      mt * mt * mt * startY +
      3 * mt * mt * t * height * 0.8 +
      3 * mt * t * t * height * 0.84 +
      t * t * t * (pondY - 4);
    fillPixelRect(
      args.ctx,
      x,
      y - 1,
      3,
      1,
      "#BFE0F0",
      0.6 * Math.sin(t * Math.PI),
    );
  }
}

export function drawRainPuddles(args: DrawBuddyWorldBaseArgs): void {
  if (worldWeather(args.world) !== "rain") return;
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const frame = safeFrame(args.frame);
  const spots = [
    { x: 0.4, y: 0.9, r: 14 },
    { x: 0.55, y: 0.94, r: 10 },
    { x: 0.68, y: 0.89, r: 12 },
  ];

  for (let index = 0; index < spots.length; index += 1) {
    const spot = spots[index];
    const px = width * spot.x;
    const py = height * spot.y;
    fillEllipse(args.ctx, px, py, spot.r, spot.r * 0.26, "#9CC3DB", 0.3);
    fillEllipse(
      args.ctx,
      px - spot.r * 0.2,
      py - 0.6,
      spot.r * 0.5,
      spot.r * 0.1,
      "#C9E2EF",
      0.26,
    );
    if (args.reducedMotion) continue;
    const ringT = ((frame * 0.6 + index * 37) % 60) / 60;
    args.ctx.save();
    args.ctx.globalAlpha = (1 - ringT) * 0.4;
    args.ctx.strokeStyle = "#BFE0F0";
    args.ctx.lineWidth = 1;
    args.ctx.beginPath();
    args.ctx.ellipse(
      px,
      py,
      spot.r * 0.2 + ringT * spot.r * 0.7,
      (spot.r * 0.2 + ringT * spot.r * 0.7) * 0.26,
      0,
      0,
      Math.PI * 2,
    );
    args.ctx.stroke();
    args.ctx.restore();
  }
}

export function drawWindStreaks(args: DrawBuddyWorldBaseArgs): void {
  const phase = worldPhase(args.world);
  if (phase === "night") return;
  const weather = worldWeather(args.world);
  if (weather !== "clear" && weather !== "wind") return;
  if (args.reducedMotion) return;
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const frame = safeFrame(args.frame);
  const gustCycle = 520;
  const within = (frame * 1.4) % gustCycle;
  if (within > 170) return;
  const t = within / 170;
  const sweep = t * width * 1.3 - width * 0.15;
  const fade = Math.sin(t * Math.PI) * 0.16;

  for (let index = 0; index < 2; index += 1) {
    const y = height * (0.34 + index * 0.18) + Math.sin(t * 6 + index * 2) * 8;
    strokeBezier(
      args.ctx,
      { x: sweep - 90, y: y + 6 },
      { x: sweep - 40, y: y - 8 },
      { x: sweep + 10, y: y + 9 },
      { x: sweep + 70, y: y - 4 },
      "#EAF6F8",
      1.6,
      fade,
    );
    fillCircle(args.ctx, sweep + 70, y - 4, 2, "#EAF6F8", fade * 0.8);
  }
}
