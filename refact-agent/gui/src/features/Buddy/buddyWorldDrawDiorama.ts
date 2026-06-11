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
  const seasonFlower =
    args.world.season === "spring"
      ? "#F9A8D4"
      : args.world.season === "autumn"
        ? "#FB923C"
        : args.world.season === "winter"
          ? "#E0F2FE"
          : "#FDE68A";
  const flowerColor =
    phase === "evening"
      ? "#FDBA74"
      : phase === "night"
        ? "#A7F3D0"
        : seasonFlower;

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
export function drawPond(args: DrawBuddyWorldBaseArgs): void {
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const frame = safeFrame(args.frame);
  const x = width * 0.13;
  const y = height * 0.875;
  const rx = args.compact ? 26 : 38;
  const ry = args.compact ? 7 : 10;
  const frozen = args.world.season === "winter";

  fillEllipse(args.ctx, x, y + 2, rx + 6, ry + 3, "#78350F", 0.4);

  if (frozen) {
    fillEllipse(args.ctx, x, y, rx, ry, "#BAE6FD", 0.78);
    fillEllipse(
      args.ctx,
      x - rx * 0.2,
      y - 1,
      rx * 0.4,
      ry * 0.34,
      "#E0F2FE",
      0.5,
    );
    strokeLine(
      args.ctx,
      { x: x - rx * 0.5, y: y - 2 },
      { x: x + rx * 0.24, y: y + 3 },
      "#E0F2FE",
      1,
      0.5,
    );
    strokeLine(
      args.ctx,
      { x: x + rx * 0.1, y: y - ry * 0.4 },
      { x: x + rx * 0.42, y: y + 2 },
      "#E0F2FE",
      1,
      0.36,
    );
    return;
  }

  fillEllipse(args.ctx, x, y, rx, ry, "#0C4A6E", 0.82);
  fillEllipse(args.ctx, x, y, rx * 0.84, ry * 0.78, "#0EA5E9", 0.4);
  const rippleSpread = wave(frame, 46, 0, 3, args.reducedMotion);
  strokeEllipse(
    args.ctx,
    x - rx * 0.2,
    y,
    rx * 0.34 + rippleSpread,
    ry * 0.3 + rippleSpread * 0.3,
    "#7DD3FC",
    1,
    alphaForMotion(0.3, args.reducedMotion),
  );
  fillEllipse(args.ctx, x + rx * 0.4, y - 2, 6, 2.6, "#16A34A", 0.85);
  fillPixelRect(args.ctx, x + rx * 0.4, y - 4, 2, 2, "#F9A8D4", 0.9);

  if (!hasWorldLayer(args.world, "pond_life") || args.reducedMotion) return;
  const within = frame % 320;
  if (within < 42) {
    const t = within / 42;
    const fx = x - 12 + t * 24;
    const fy = y - Math.sin(t * Math.PI) * 13;
    fillPixelRect(args.ctx, fx, fy, 4, 2, "#FB923C", 0.92);
    fillPixelRect(args.ctx, fx - 2, fy + 1, 2, 1, "#FDBA74", 0.8);
    if (t < 0.16 || t > 0.84) {
      fillPixelRect(args.ctx, fx + 1, y - 1, 1, 1, "#BAE6FD", 0.8);
      fillPixelRect(args.ctx, fx - 2, y - 2, 1, 1, "#BAE6FD", 0.66);
    }
  }
}

export function drawLanterns(args: DrawBuddyWorldBaseArgs): void {
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const frame = safeFrame(args.frame);
  const lit = hasWorldLayer(args.world, "lanterns");
  const posts = [
    { x: width * 0.36, y: height * 0.79 },
    { x: width * 0.43, y: height * 0.815 },
    { x: width * 0.5, y: height * 0.84 },
  ];

  for (let index = 0; index < posts.length; index += 1) {
    const post = posts[index];
    fillPixelRect(args.ctx, post.x, post.y - 12, 2, 14, "#52525B", 0.9);
    fillPixelRect(args.ctx, post.x - 2, post.y - 17, 6, 6, "#1E293B", 0.92);
    if (lit) {
      const flicker = wave(
        frame,
        22 + index * 3,
        index,
        0.05,
        args.reducedMotion,
      );
      fillPixelRect(args.ctx, post.x - 1, post.y - 16, 4, 4, "#FDE68A", 0.92);
      fillCircle(
        args.ctx,
        post.x + 1,
        post.y - 14,
        args.compact ? 9 : 13,
        "#FBBF24",
        alphaForMotion(0.12 + flicker, args.reducedMotion),
      );
    } else {
      fillPixelRect(args.ctx, post.x - 1, post.y - 16, 4, 4, "#475569", 0.85);
    }
  }
}

export function drawCampfire(args: DrawBuddyWorldBaseArgs): void {
  if (!hasWorldLayer(args.world, "campfire")) return;
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const frame = safeFrame(args.frame);
  const x = width * 0.585;
  const y = height * 0.865;

  fillCircle(
    args.ctx,
    x,
    y - 4,
    args.compact ? 18 : 26,
    "#FB923C",
    alphaForMotion(
      0.1 + wave(frame, 26, 0, 0.04, args.reducedMotion),
      args.reducedMotion,
    ),
  );
  fillPixelRect(args.ctx, x - 8, y, 16, 3, "#7C2D12", 0.95);
  fillPixelRect(args.ctx, x - 6, y + 2, 12, 2, "#92400E", 0.9);
  const flame = args.reducedMotion ? 0 : Math.abs(Math.sin(frame / 9));
  const outerH = 9 + flame * 4;
  const midH = 6 + flame * 3;
  fillPixelRect(args.ctx, x - 3, y - outerH, 6, outerH, "#FB923C", 0.85);
  fillPixelRect(args.ctx, x - 2, y - midH, 4, midH, "#FDBA74", 0.9);
  fillPixelRect(
    args.ctx,
    x - 1,
    y - 3 - flame * 2,
    2,
    3 + flame * 2,
    "#FEF3C7",
    0.95,
  );

  if (!args.reducedMotion) {
    const emberCount = countForMotion(3, args.compact, false);
    for (let index = 0; index < emberCount; index += 1) {
      const cycle = (frame * (0.6 + index * 0.2) + index * 47) % 60;
      fillPixelRect(
        args.ctx,
        x -
          3 +
          ((index * 5 + frame / 9) % 7) +
          Math.sin(frame / 11 + index) * 2,
        y - 8 - cycle * 0.5,
        1.6,
        1.6,
        "#FDE68A",
        Math.max(0, 0.7 - cycle / 60),
      );
    }
    fillCircle(
      args.ctx,
      x + 2,
      y - outerH - 13 - (frame % 40) * 0.3,
      4,
      "#94A3B8",
      0.07,
    );
    fillCircle(
      args.ctx,
      x - 1,
      y - outerH - 22 - (frame % 40) * 0.3,
      5,
      "#94A3B8",
      0.05,
    );
  }
}

export function drawMailbox(args: DrawBuddyWorldBaseArgs): void {
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const frame = safeFrame(args.frame);
  const x = pctX(width, BUDDY_WORLD_HOME_HOTSPOT.x) + (args.compact ? 30 : 42);
  const y = pctY(height, BUDDY_WORLD_HOME_HOTSPOT.y) + 18;
  const flagUp = hasWorldLayer(args.world, "quest_mailbox");

  fillPixelRect(args.ctx, x, y - 6, 2, 14, "#713F12", 0.95);
  fillPixelRect(args.ctx, x - 5, y - 12, 12, 7, "#DC2626", 0.95);
  fillPixelRect(args.ctx, x - 5, y - 12, 12, 2, "#B91C1C", 0.95);
  fillPixelRect(args.ctx, x - 4, y - 9, 4, 3, "#1E293B", 0.9);
  if (flagUp) {
    fillPixelRect(args.ctx, x + 7, y - 18, 2, 7, "#FDE047", 0.96);
    fillPixelRect(args.ctx, x + 7, y - 18, 5, 3, "#FDE047", 0.96);
    const pulse = wave(frame, 24, 0, 0.2, args.reducedMotion);
    fillCircle(args.ctx, x + 2, y - 12, 10, "#FDE047", 0.08 + pulse * 0.3);
  } else {
    fillPixelRect(args.ctx, x + 7, y - 11, 6, 2, "#A16207", 0.9);
  }
}

export function drawWinterGroundDust(args: DrawBuddyWorldBaseArgs): void {
  if (args.world.season !== "winter") return;
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const baseY = height * 0.8;

  for (let x = 14; x < width; ) {
    const offset = (x * 13) % 53;
    const px = x + offset;
    const py = baseY + 8 + ((x * 7) % 26);
    fillEllipse(args.ctx, px, py, 12 + ((x * 11) % 9), 3, "#F8FAFC", 0.14);
    x += (args.compact || args.reducedMotion ? 120 : 88) + offset;
  }
}
