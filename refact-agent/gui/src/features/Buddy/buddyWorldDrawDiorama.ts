import {
  BUDDY_WORLD_HOME_HOTSPOT,
  alphaForMotion,
  countForMotion,
  fillCircle,
  fillEllipse,
  fillPixelRect,
  finiteOr,
  hasWorldLayer,
  pctX,
  pctY,
  safeDimension,
  safeFrame,
  strokeLine,
  strokeEllipse,
  wave,
  worldIntensity,
  worldPhase,
  type DrawBuddyWorldBaseArgs,
} from "./buddyWorldDrawHelpers";

export function drawDistantHills(args: DrawBuddyWorldBaseArgs): void {
  const { ctx, world } = args;
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const frame = safeFrame(args.frame);
  const farY = height * 0.62;
  const nearY = height * 0.69;
  const phase = worldPhase(world);
  const farColor = phase === "night" ? "#1E3A5F" : "#2F855A";
  const nearColor = phase === "night" ? "#155E49" : "#166534";

  ctx.save();
  ctx.fillStyle = `${farColor}66`;
  ctx.beginPath();
  ctx.moveTo(0, farY + 18);
  for (let x = 0; x <= width; x += 20) {
    const y = farY + wave(frame, 210, x / 56, 8, args.reducedMotion);
    ctx.lineTo(x, y);
  }
  ctx.lineTo(width, height);
  ctx.lineTo(0, height);
  ctx.closePath();
  ctx.fill();

  ctx.fillStyle = `${nearColor}88`;
  ctx.beginPath();
  ctx.moveTo(0, nearY + 16);
  for (let x = 0; x <= width; x += 16) {
    const y =
      nearY +
      wave(frame, 180, x / 42, 6, args.reducedMotion) +
      Math.sin(finiteOr(x, 0) / 19) * 2;
    ctx.lineTo(x, y);
  }
  ctx.lineTo(width, height);
  ctx.lineTo(0, height);
  ctx.closePath();
  ctx.fill();
  const horizonColor =
    phase === "morning"
      ? "#FDE68A"
      : phase === "evening"
        ? "#FB7185"
        : phase === "night"
          ? "#818CF8"
          : "#BBF7D0";
  fillEllipse(
    ctx,
    width * 0.5,
    farY + 18,
    width * (phase === "day" ? 0.28 : 0.34),
    18,
    horizonColor,
    alphaForMotion(phase === "night" ? 0.08 : 0.14, args.reducedMotion),
  );
  ctx.restore();
}

export function drawMidgroundGarden(args: DrawBuddyWorldBaseArgs): void {
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const frame = safeFrame(args.frame);
  const gardenY = height * 0.69;
  const count = countForMotion(18, args.compact, args.reducedMotion);
  const phase = worldPhase(args.world);
  const flowerColor =
    phase === "evening" ? "#FDBA74" : phase === "night" ? "#A7F3D0" : "#FDE68A";

  fillEllipse(
    args.ctx,
    width * 0.32,
    gardenY + 18,
    width * 0.18,
    args.compact ? 12 : 18,
    phase === "night" ? "#2DD4BF" : "#86EFAC",
    alphaForMotion(phase === "day" ? 0.2 : 0.12, args.reducedMotion),
  );

  for (let index = 0; index < count; index += 1) {
    const x = (index / count) * width + ((index * 17) % 23);
    const stem = 8 + ((index * 7) % 12);
    const sway = wave(frame, 40, index, 2.5, args.reducedMotion);
    fillPixelRect(args.ctx, x + sway, gardenY + 7, 3, stem, "#166534", 0.54);
    fillPixelRect(args.ctx, x - 5 + sway, gardenY + 8, 11, 3, "#4ADE80", 0.34);
    if (index % 4 === 0) {
      fillPixelRect(
        args.ctx,
        x + 1 + sway,
        gardenY + 3,
        4,
        4,
        flowerColor,
        0.46,
      );
    }
  }
}

export function drawWorkshopZones(args: DrawBuddyWorldBaseArgs): void {
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const frame = safeFrame(args.frame);
  const active = hasWorldLayer(args.world, "workshop_runes");
  const memoryActive = hasWorldLayer(args.world, "memory_orbs");
  const alpha = alphaForMotion(active ? 0.24 : 0.16, args.reducedMotion);
  const intensity = worldIntensity(args.world);

  fillEllipse(
    args.ctx,
    width * 0.62,
    height * 0.72,
    width * 0.16,
    14,
    "#0F172A",
    alpha,
  );
  fillEllipse(
    args.ctx,
    width * 0.34,
    height * 0.69,
    width * 0.12,
    10,
    "#422006",
    alpha * 0.82,
  );
  fillEllipse(
    args.ctx,
    width * 0.78,
    height * 0.64,
    width * 0.1,
    9,
    "#1E1B4B",
    alpha * 0.78,
  );

  fillPixelRect(args.ctx, width * 0.28, height * 0.58, 48, 32, "#422006", 0.74);
  fillPixelRect(
    args.ctx,
    width * 0.285,
    height * 0.6,
    40,
    4,
    "#FDE68A",
    memoryActive ? 0.46 : 0.24,
  );
  fillPixelRect(
    args.ctx,
    width * 0.285,
    height * 0.65,
    40,
    4,
    "#FDE68A",
    memoryActive ? 0.4 : 0.18,
  );
  fillPixelRect(
    args.ctx,
    width * 0.615,
    height * 0.59,
    58,
    43,
    "#1E293B",
    0.72,
  );
  fillPixelRect(
    args.ctx,
    width * 0.625,
    height * 0.54,
    38,
    12,
    active ? "#60A5FA" : "#475569",
    active ? 0.8 : 0.62,
  );

  for (let index = 0; index < 5; index += 1) {
    const x = width * 0.56 + index * width * 0.026;
    const y =
      height * 0.69 +
      wave(frame, 32, index, active ? 4 : 2, args.reducedMotion);
    fillPixelRect(
      args.ctx,
      x,
      y,
      4,
      15 - index,
      index % 2 === 0 ? "#60A5FA" : "#A78BFA",
      active ? 0.34 : 0.2,
    );
  }

  if (active) {
    const portalX = width * 0.67;
    const portalY = height * 0.67;
    strokeEllipse(
      args.ctx,
      portalX,
      portalY,
      args.compact ? 24 : 34,
      args.compact ? 13 : 18,
      "#67E8F9",
      3,
      alpha * 0.9,
    );
    strokeEllipse(
      args.ctx,
      portalX,
      portalY,
      args.compact ? 15 : 22,
      args.compact ? 8 : 12,
      "#FDE68A",
      2,
      alpha * 0.72,
    );
    strokeLine(
      args.ctx,
      { x: portalX - 58, y: portalY + 14 },
      {
        x: portalX + 42,
        y: portalY - 18 + wave(frame, 54, 0, 7, args.reducedMotion),
      },
      "#A78BFA",
      args.compact ? 2 : 3,
      alphaForMotion(0.1 + intensity * 0.08, args.reducedMotion),
    );
  }
}

export function drawGround(args: DrawBuddyWorldBaseArgs): void {
  const { ctx } = args;
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const frame = safeFrame(args.frame);
  const baseY = height * 0.745;

  ctx.save();
  ctx.fillStyle = "rgba(22, 101, 52, 0.46)";
  ctx.beginPath();
  ctx.moveTo(0, baseY + 10);
  for (let x = 0; x <= width; x += 18) {
    const hill =
      wave(frame, 150, x / 48, 7, args.reducedMotion) +
      Math.sin(finiteOr(x, 0) / 19) * 2;
    ctx.lineTo(x, baseY + hill);
  }
  ctx.lineTo(width, height);
  ctx.lineTo(0, height);
  ctx.closePath();
  ctx.fill();
  ctx.restore();

  for (let x = 0; x < width; x += 8) {
    const ridge =
      wave(frame, 110, x / 39, 3, args.reducedMotion) +
      Math.sin(finiteOr(x, 0) / 23) * 2;
    fillPixelRect(
      ctx,
      x,
      baseY + ridge,
      8,
      height - baseY - ridge,
      "rgba(20,83,45,0.88)",
    );
    if ((x / 8) % 11 === 0) {
      fillPixelRect(
        ctx,
        x + 2,
        baseY + ridge + 11,
        7,
        2,
        "rgba(74,222,128,0.2)",
      );
    }
  }

  const grassStep = args.compact || args.reducedMotion ? 82 : 52;
  for (let x = 0; x < width; ) {
    const offset = (x * 17) % 43;
    const clumpX = x + offset;
    const clumpY = baseY + 12 + ((x * 11) % 22);
    const grassHeight = 8 + wave(frame, 64, x + offset, 4, args.reducedMotion);
    fillPixelRect(
      ctx,
      clumpX,
      clumpY - grassHeight,
      3,
      grassHeight,
      "rgba(187,247,208,0.28)",
    );
    fillPixelRect(
      ctx,
      clumpX + 4,
      clumpY - grassHeight + 2,
      2,
      Math.max(2, grassHeight - 1),
      "rgba(74,222,128,0.24)",
    );
    x += grassStep + offset;
  }
}

export function drawHomePath(args: DrawBuddyWorldBaseArgs): void {
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const frame = safeFrame(args.frame);
  const startX = pctX(width, BUDDY_WORLD_HOME_HOTSPOT.x) + 28;
  const startY = pctY(height, BUDDY_WORLD_HOME_HOTSPOT.y) + 38;
  const endX = width / 2;
  const endY = height * 0.84;
  const steps = args.compact ? 9 : 12;

  for (let index = 0; index < steps; index += 1) {
    const t = index / (steps - 1);
    const x = startX + (endX - startX) * t + Math.sin(index * 1.6) * 5;
    const y =
      startY +
      (endY - startY) * t +
      wave(frame, 50, index, 1.1, args.reducedMotion);
    fillEllipse(args.ctx, x, y, 8 - t * 2, 3.2, "#92400E", 0.32 - t * 0.1);
  }
}

export function drawBuddyLandingPad(args: DrawBuddyWorldBaseArgs): void {
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const x = width / 2;
  const y = height * 0.735 + wave(args.frame, 30, 0, 1.2, args.reducedMotion);

  fillEllipse(args.ctx, x, y + 18, args.compact ? 48 : 62, 13, "#041412", 0.32);
  fillEllipse(args.ctx, x, y + 14, args.compact ? 34 : 44, 8, "#4ADE80", 0.16);
  strokeEllipse(
    args.ctx,
    x,
    y + 13,
    args.compact ? 27 : 33,
    6,
    "#BBF7D0",
    1,
    0.12,
  );
}

export function drawVitality(args: DrawBuddyWorldBaseArgs): void {
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const frame = safeFrame(args.frame);
  const groundY = height * 0.8;

  if (args.world.vitality === "lush") {
    for (let x = 8; x < width; ) {
      const offset = (x * 11) % 37;
      const bloomY = groundY + 16 + wave(frame, 30, x, 2, args.reducedMotion);
      fillPixelRect(args.ctx, x + 10, bloomY, 4, 4, "#FDE68A");
      fillPixelRect(args.ctx, x + 14, bloomY - 4, 4, 4, "#F9A8D4");
      fillPixelRect(args.ctx, x + 14, bloomY + 4, 4, 4, "#86EFAC");
      x += (args.compact || args.reducedMotion ? 76 : 56) + offset;
    }
    return;
  }

  if (args.world.vitality === "growing") {
    for (let x = 20; x < width; ) {
      const offset = (x * 13) % 41;
      const sway = wave(frame, 32, x, 2, args.reducedMotion);
      fillPixelRect(args.ctx, x + sway, groundY + 12, 4, 18, "#16A34A");
      fillPixelRect(args.ctx, x - 9 + sway, groundY + 14, 12, 5, "#86EFAC");
      fillPixelRect(args.ctx, x + 3 + sway, groundY + 8, 14, 5, "#4ADE80");
      x += (args.compact || args.reducedMotion ? 88 : 66) + offset;
    }
    return;
  }

  for (let x = 18; x < width; ) {
    const offset = (x * 13) % 47;
    const clumpX = x + offset;
    const sway = wave(frame, 36, clumpX, 3, args.reducedMotion);
    const heightOffset = (x * 7) % 10;
    fillPixelRect(
      args.ctx,
      clumpX + sway,
      groundY + 10 - heightOffset,
      4,
      18,
      "#365314",
    );
    fillPixelRect(args.ctx, clumpX - 6 + sway, groundY + 16, 12, 3, "#854D0E");
    if (x % 5 === 0) {
      fillPixelRect(
        args.ctx,
        clumpX + 8 + sway,
        groundY + 22,
        3,
        3,
        "#EF4444",
        0.58,
      );
    }
    x += (args.compact || args.reducedMotion ? 132 : 104) + offset;
  }
}

export function drawForegroundCozyDetails(args: DrawBuddyWorldBaseArgs): void {
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const frame = safeFrame(args.frame);
  const count = countForMotion(11, args.compact, args.reducedMotion);

  for (let index = 0; index < count; index += 1) {
    const x = (index / count) * width + ((index * 23) % 31);
    const y = height * 0.9 + ((index * 7) % 18);
    const alpha = 0.16 + ((index * 13) % 8) / 100;
    fillCircle(
      args.ctx,
      x,
      y + wave(frame, 64, index, 1.2, args.reducedMotion),
      2.6,
      "#BBF7D0",
      alpha,
    );
  }
}
