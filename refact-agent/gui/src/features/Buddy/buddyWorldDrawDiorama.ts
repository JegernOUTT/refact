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
  worldPaletteHint,
  worldPhase,
  type DrawBuddyWorldBaseArgs,
} from "./buddyWorldDrawHelpers";

interface HillTints {
  far: string;
  near: string;
  glow: string;
  glowAlpha: number;
}

function hillTints(args: DrawBuddyWorldBaseArgs): HillTints {
  const hint = worldPaletteHint(args.world);
  const winter = args.world.season === "winter";
  if (hint === "night" || hint === "dream") {
    return winter
      ? { far: "#33506B", near: "#27415C", glow: "#818CF8", glowAlpha: 0.07 }
      : { far: "#2A4258", near: "#20364C", glow: "#818CF8", glowAlpha: 0.08 };
  }
  if (hint === "storm") {
    return winter
      ? { far: "#75879A", near: "#5F7287", glow: "#93A2B2", glowAlpha: 0.06 }
      : { far: "#5E7270", near: "#49605C", glow: "#93A2B2", glowAlpha: 0.06 };
  }
  if (hint === "dusk") {
    return winter
      ? { far: "#9C92B4", near: "#7E7AA0", glow: "#FB7185", glowAlpha: 0.12 }
      : { far: "#7D7C9A", near: "#5F7479", glow: "#FB7185", glowAlpha: 0.13 };
  }
  if (hint === "dawn") {
    return winter
      ? { far: "#AEBCD2", near: "#92A8C2", glow: "#FDE68A", glowAlpha: 0.13 }
      : { far: "#8FAE92", near: "#6A9A6E", glow: "#FDE68A", glowAlpha: 0.14 };
  }
  return winter
    ? { far: "#AFC4D6", near: "#93ACC2", glow: "#BBF7D0", glowAlpha: 0.1 }
    : { far: "#7FB08A", near: "#5B9466", glow: "#BBF7D0", glowAlpha: 0.11 };
}

export function drawDistantHills(args: DrawBuddyWorldBaseArgs): void {
  const { ctx } = args;
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const frame = safeFrame(args.frame);
  const tints = hillTints(args);
  const farY = height * 0.655;
  const nearY = height * 0.7;
  const breathe = wave(frame, 240, 0, 1.4, args.reducedMotion);

  ctx.save();
  ctx.globalAlpha = alphaForMotion(0.66, args.reducedMotion);
  ctx.fillStyle = tints.far;
  ctx.beginPath();
  ctx.moveTo(0, farY + 6);
  for (let x = 0; x <= width; x += 14) {
    const t = x / width;
    const mound =
      Math.pow(Math.abs(Math.sin(t * Math.PI * 2.1 + 0.6)), 1.4) * 16 +
      Math.sin(t * Math.PI * 5.3 + 2.2) * 3;
    ctx.lineTo(x, farY - mound + breathe * 0.4);
  }
  ctx.lineTo(width, height);
  ctx.lineTo(0, height);
  ctx.closePath();
  ctx.fill();
  ctx.restore();

  fillEllipse(
    ctx,
    width * 0.5,
    farY - 4,
    width * 0.4,
    10,
    tints.glow,
    alphaForMotion(tints.glowAlpha, args.reducedMotion),
  );

  ctx.save();
  ctx.globalAlpha = alphaForMotion(0.78, args.reducedMotion);
  ctx.fillStyle = tints.near;
  ctx.beginPath();
  ctx.moveTo(0, nearY + 8);
  for (let x = 0; x <= width; x += 12) {
    const t = x / width;
    const mound =
      Math.pow(Math.abs(Math.sin(t * Math.PI * 1.6 + 2.4)), 1.3) * 13 +
      Math.sin(t * Math.PI * 4.1 + 0.8) * 2.4;
    ctx.lineTo(x, nearY - mound - breathe * 0.3);
  }
  ctx.lineTo(width, height);
  ctx.lineTo(0, height);
  ctx.closePath();
  ctx.fill();
  ctx.restore();
}

interface GardenTints {
  band: string;
  bandAlpha: number;
  stem: string;
  blade: string;
}

function gardenTints(args: DrawBuddyWorldBaseArgs): GardenTints {
  const hint = worldPaletteHint(args.world);
  if (args.world.season === "winter") {
    const nightish = hint === "night" || hint === "dream";
    return {
      band: nightish ? "#46607E" : "#E8F1F8",
      bandAlpha: nightish ? 0.14 : 0.22,
      stem: nightish ? "#6E7E90" : "#9D8C6C",
      blade: nightish ? "#5C6C7E" : "#B5A37E",
    };
  }
  if (hint === "night" || hint === "dream") {
    return {
      band: "#2DD4BF",
      bandAlpha: 0.12,
      stem: "#1F5E48",
      blade: "#3D7E62",
    };
  }
  if (hint === "dusk") {
    return {
      band: "#C99A5E",
      bandAlpha: 0.14,
      stem: "#5C6E3C",
      blade: "#8A965C",
    };
  }
  if (hint === "dawn") {
    return {
      band: "#BCD49A",
      bandAlpha: 0.16,
      stem: "#3E7A48",
      blade: "#74A86A",
    };
  }
  if (hint === "storm") {
    return {
      band: "#6F8A74",
      bandAlpha: 0.12,
      stem: "#3A6244",
      blade: "#5E8862",
    };
  }
  return { band: "#9CD478", bandAlpha: 0.2, stem: "#3E8048", blade: "#6FAE6A" };
}

export function drawMidgroundGarden(args: DrawBuddyWorldBaseArgs): void {
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const frame = safeFrame(args.frame);
  const gardenY = height * 0.69;
  const count = countForMotion(18, args.compact, args.reducedMotion);
  const phase = worldPhase(args.world);
  const tints = gardenTints(args);
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
    tints.band,
    alphaForMotion(tints.bandAlpha, args.reducedMotion),
  );

  for (let index = 0; index < count; index += 1) {
    const x = (index / count) * width + ((index * 17) % 23);
    const stem = 8 + ((index * 7) % 12);
    const sway = args.reducedMotion
      ? 0
      : Math.sin(frame / 26 - x / 55) * 2.6 +
        Math.sin(frame / 11 + index) * 0.8;
    fillPixelRect(args.ctx, x + sway, gardenY + 7, 3, stem, tints.stem, 0.6);
    fillPixelRect(args.ctx, x - 5 + sway, gardenY + 8, 11, 3, tints.blade, 0.4);
    if (index % 4 === 0) {
      fillPixelRect(
        args.ctx,
        x + 1 + sway * 1.3,
        gardenY + 3,
        4,
        4,
        flowerColor,
        0.52,
      );
      fillPixelRect(
        args.ctx,
        x + 2 + sway * 1.3,
        gardenY + 4,
        2,
        2,
        "#FFF7E0",
        0.4,
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
  const hint = worldPaletteHint(args.world);
  const nightish = hint === "night" || hint === "dream";
  const stone = nightish ? "#55607A" : hint === "dusk" ? "#8E8290" : "#9A8E80";
  const stoneShade = nightish
    ? "#3A4458"
    : hint === "dusk"
      ? "#675E6E"
      : "#6E6456";
  const mossTop = nightish
    ? "#2C5638"
    : hint === "dusk"
      ? "#3F6340"
      : "#4F8F54";
  const logWood = nightish ? "#3A2C20" : "#6B4F3A";
  const logShade = nightish ? "#241A14" : "#4A362A";

  fillEllipse(
    args.ctx,
    width * 0.62,
    height * 0.72,
    width * 0.14,
    12,
    "#13231A",
    alpha,
  );

  const bx = width * 0.62;
  const by = height * 0.685;
  fillEllipse(args.ctx, bx, by, 21, 13, stone, 0.94);
  fillEllipse(args.ctx, bx - 5, by - 4, 12, 7, stone, 0.96);
  fillEllipse(args.ctx, bx + 3, by - 8, 13, 5, mossTop, 0.85);
  fillEllipse(args.ctx, bx - 9, by - 9, 7, 3.4, mossTop, 0.75);
  fillEllipse(args.ctx, bx + 7, by + 6, 9, 5, stoneShade, 0.6);
  fillEllipse(args.ctx, bx + 24, by + 7, 8, 5, stone, 0.9);
  fillEllipse(args.ctx, bx + 24, by + 4.4, 6, 2.6, mossTop, 0.7);

  const lx = width * 0.545;
  const ly = height * 0.775;
  fillEllipse(args.ctx, lx, ly + 6, 28, 4, "#13231A", alpha * 0.9);
  fillPixelRect(args.ctx, lx - 26, ly - 3, 50, 9, logWood, 0.95);
  fillPixelRect(args.ctx, lx - 26, ly + 4, 50, 2.4, logShade, 0.9);
  fillEllipse(args.ctx, lx + 25, ly + 1.4, 4.4, 5.4, logShade, 0.96);
  fillEllipse(args.ctx, lx + 25, ly + 1.4, 2.4, 3, "#8A6A4F", 0.9);
  fillPixelRect(args.ctx, lx - 18, ly - 5.4, 9, 2.6, mossTop, 0.8);
  fillPixelRect(args.ctx, lx - 2, ly - 5, 7, 2.4, mossTop, 0.7);
  const shroomGlow = memoryActive ? 0.66 : 0.34;
  fillPixelRect(args.ctx, lx + 8, ly - 6.4, 2, 3.4, "#E2CFAE", 0.9);
  fillPixelRect(args.ctx, lx + 6.6, ly - 8.4, 5, 2.6, "#FDE68A", shroomGlow);
  fillPixelRect(args.ctx, lx - 11, ly - 7, 1.8, 3, "#E2CFAE", 0.85);
  fillPixelRect(
    args.ctx,
    lx - 12.4,
    ly - 9,
    4.4,
    2.4,
    "#FDE68A",
    shroomGlow * 0.9,
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
    const ringX = width * 0.67;
    const ringY = height * 0.74;
    strokeEllipse(
      args.ctx,
      ringX,
      ringY,
      args.compact ? 24 : 34,
      args.compact ? 8 : 11,
      "#67E8F9",
      3,
      alpha * 0.9,
    );
    strokeEllipse(
      args.ctx,
      ringX,
      ringY,
      args.compact ? 15 : 22,
      args.compact ? 5 : 7,
      "#FDE68A",
      2,
      alpha * 0.72,
    );
    strokeLine(
      args.ctx,
      { x: ringX - 58, y: ringY + 10 },
      {
        x: ringX + 42,
        y: ringY - 22 + wave(frame, 54, 0, 7, args.reducedMotion),
      },
      "#A78BFA",
      args.compact ? 2 : 3,
      alphaForMotion(0.1 + intensity * 0.08, args.reducedMotion),
    );
  }
}

interface GroundTints {
  base: string;
  baseAlpha: number;
  ridge: string;
  ridgeAlpha: number;
  fleck: string;
  fleckAlpha: number;
  pebble: string;
  pebbleAlpha: number;
  grassA: string;
  grassAAlpha: number;
  grassB: string;
  grassBAlpha: number;
  grassC: string;
  grassCAlpha: number;
  flowers: boolean;
}

function groundTints(args: DrawBuddyWorldBaseArgs): GroundTints {
  const hint = worldPaletteHint(args.world);
  const nightish = hint === "night" || hint === "dream";
  if (args.world.season === "winter") {
    if (nightish) {
      return {
        base: "#3E5570",
        baseAlpha: 0.62,
        ridge: "#34485E",
        ridgeAlpha: 0.94,
        fleck: "#5E7A96",
        fleckAlpha: 0.3,
        pebble: "#28384A",
        pebbleAlpha: 0.4,
        grassA: "#8C8268",
        grassAAlpha: 0.3,
        grassB: "#766C56",
        grassBAlpha: 0.26,
        grassC: "#5E563F",
        grassCAlpha: 0.2,
        flowers: false,
      };
    }
    return {
      base: "#DCE8F2",
      baseAlpha: 0.78,
      ridge: hint === "dusk" ? "#C3C2DC" : "#C7D8E8",
      ridgeAlpha: 0.95,
      fleck: "#F2F7FB",
      fleckAlpha: 0.5,
      pebble: "#A9BDD0",
      pebbleAlpha: 0.5,
      grassA: "#C9B68C",
      grassAAlpha: 0.4,
      grassB: "#B5A37E",
      grassBAlpha: 0.34,
      grassC: "#9D8C6C",
      grassCAlpha: 0.26,
      flowers: false,
    };
  }
  if (nightish) {
    return {
      base: "#1E4A44",
      baseAlpha: 0.52,
      ridge: "#173A3C",
      ridgeAlpha: 0.9,
      fleck: "#62A696",
      fleckAlpha: 0.18,
      pebble: "#48807F",
      pebbleAlpha: 0.3,
      grassA: "#96CDBA",
      grassAAlpha: 0.26,
      grassB: "#6AA694",
      grassBAlpha: 0.22,
      grassC: "#588E80",
      grassCAlpha: 0.18,
      flowers: false,
    };
  }
  if (hint === "dusk") {
    return {
      base: "#9C8A52",
      baseAlpha: 0.52,
      ridge: "#7C7050",
      ridgeAlpha: 0.9,
      fleck: "#D9BD86",
      fleckAlpha: 0.24,
      pebble: "#5E5440",
      pebbleAlpha: 0.4,
      grassA: "#E0C896",
      grassAAlpha: 0.32,
      grassB: "#B6A070",
      grassBAlpha: 0.28,
      grassC: "#94855C",
      grassCAlpha: 0.22,
      flowers: true,
    };
  }
  if (hint === "dawn") {
    return {
      base: "#8FAC60",
      baseAlpha: 0.52,
      ridge: "#73935A",
      ridgeAlpha: 0.9,
      fleck: "#C9D9A2",
      fleckAlpha: 0.26,
      pebble: "#4E6840",
      pebbleAlpha: 0.4,
      grassA: "#DCE8B0",
      grassAAlpha: 0.34,
      grassB: "#B2C588",
      grassBAlpha: 0.3,
      grassC: "#94AC6E",
      grassCAlpha: 0.24,
      flowers: true,
    };
  }
  if (hint === "storm") {
    return {
      base: "#4E6650",
      baseAlpha: 0.52,
      ridge: "#3C5444",
      ridgeAlpha: 0.9,
      fleck: "#8CA890",
      fleckAlpha: 0.2,
      pebble: "#2E4234",
      pebbleAlpha: 0.4,
      grassA: "#A4C2A8",
      grassAAlpha: 0.28,
      grassB: "#84A48A",
      grassBAlpha: 0.24,
      grassC: "#6A8A70",
      grassCAlpha: 0.2,
      flowers: false,
    };
  }
  return {
    base: "#8FBC62",
    baseAlpha: 0.54,
    ridge: "#6CA34E",
    ridgeAlpha: 0.9,
    fleck: "#A9D87E",
    fleckAlpha: 0.28,
    pebble: "#477A3E",
    pebbleAlpha: 0.42,
    grassA: "#C9E8A6",
    grassAAlpha: 0.36,
    grassB: "#9CCB78",
    grassBAlpha: 0.3,
    grassC: "#7FB45E",
    grassCAlpha: 0.24,
    flowers: true,
  };
}

export function drawGround(args: DrawBuddyWorldBaseArgs): void {
  const { ctx } = args;
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const frame = safeFrame(args.frame);
  const baseY = height * 0.745;
  const tints = groundTints(args);

  ctx.save();
  ctx.globalAlpha = tints.baseAlpha;
  ctx.fillStyle = tints.base;
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
      tints.ridge,
      tints.ridgeAlpha,
    );
    if ((x / 8) % 11 === 0) {
      fillPixelRect(
        ctx,
        x + 2,
        baseY + ridge + 11,
        7,
        2,
        tints.fleck,
        tints.fleckAlpha,
      );
    }
    if ((x / 8) % 7 === 3) {
      fillPixelRect(
        ctx,
        x + 4,
        baseY + ridge + 17,
        2,
        2,
        tints.pebble,
        tints.pebbleAlpha,
      );
    }
  }

  const grassStep = args.compact || args.reducedMotion ? 64 : 38;
  for (let x = 0; x < width; ) {
    const offset = (x * 17) % 43;
    const clumpX = x + offset;
    const clumpY = baseY + 12 + ((x * 11) % 22);
    const gust = args.reducedMotion
      ? 0
      : Math.sin(frame / 26 - clumpX / 55) * 3 +
        Math.sin(frame / 9 - clumpX / 23) * 1.1;
    const grassHeight = 9 + gust;
    const lean = args.reducedMotion ? 0 : Math.round(gust * 0.7);
    fillPixelRect(
      ctx,
      clumpX + lean,
      clumpY - grassHeight,
      3,
      grassHeight,
      tints.grassA,
      tints.grassAAlpha,
    );
    fillPixelRect(
      ctx,
      clumpX + 4 + lean,
      clumpY - grassHeight + 2,
      2,
      Math.max(2, grassHeight - 1),
      tints.grassB,
      tints.grassBAlpha,
    );
    fillPixelRect(
      ctx,
      clumpX - 3 + Math.round(lean * 0.6),
      clumpY - grassHeight + 4,
      2,
      Math.max(2, grassHeight - 4),
      tints.grassC,
      tints.grassCAlpha,
    );
    if (tints.flowers && (clumpX | 0) % 5 === 0) {
      fillPixelRect(
        ctx,
        clumpX + 6 + lean,
        clumpY - grassHeight - 2,
        2,
        2,
        "#F6F8F4",
        0.8,
      );
      fillPixelRect(
        ctx,
        clumpX + 6.6 + lean,
        clumpY - grassHeight - 1.4,
        0.8,
        0.8,
        "#F2CD5C",
        0.9,
      );
    }
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
  const dot = args.world.season === "winter" ? "#E4EEF6" : "#BBF7D0";

  for (let index = 0; index < count; index += 1) {
    const x = (index / count) * width + ((index * 23) % 31);
    const y = height * 0.9 + ((index * 7) % 18);
    const alpha = 0.16 + ((index * 13) % 8) / 100;
    fillCircle(
      args.ctx,
      x,
      y + wave(frame, 64, index, 1.2, args.reducedMotion),
      2.6,
      dot,
      alpha,
    );
  }
}
export function drawPond(args: DrawBuddyWorldBaseArgs): void {
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const frame = safeFrame(args.frame);
  const x = width * 0.38;
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

export function drawLanterns(
  args: DrawBuddyWorldBaseArgs,
  litCountOverride?: number | null,
): void {
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const frame = safeFrame(args.frame);
  const layerLit = hasWorldLayer(args.world, "lanterns");
  const hasOverride =
    typeof litCountOverride === "number" && Number.isFinite(litCountOverride);
  const litCount = hasOverride
    ? Math.max(0, Math.min(3, Math.floor(litCountOverride)))
    : layerLit
      ? 3
      : 0;
  const posts = [
    { x: width * 0.36, y: height * 0.79 },
    { x: width * 0.43, y: height * 0.815 },
    { x: width * 0.5, y: height * 0.84 },
  ];

  for (let index = 0; index < posts.length; index += 1) {
    const post = posts[index];
    fillPixelRect(args.ctx, post.x, post.y - 12, 2, 14, "#52525B", 0.9);
    fillPixelRect(args.ctx, post.x - 2, post.y - 17, 6, 6, "#1E293B", 0.92);
    if (index < litCount) {
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

export function drawPondReflection(args: DrawBuddyWorldBaseArgs): void {
  if (
    typeof args.actorXPercent !== "number" ||
    !Number.isFinite(args.actorXPercent)
  ) {
    return;
  }
  if (Math.abs(args.actorXPercent - 38) > 9) return;
  if (args.world.season === "winter") return;
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const frame = safeFrame(args.frame);
  const x = pctX(width, args.actorXPercent);
  const y = pctY(height, 88.5);
  const ripple = wave(frame, 9, 0, 1.3, args.reducedMotion);

  fillEllipse(args.ctx, x, y, 9 + ripple, 3.1, "#1F2937", 0.16);
  fillEllipse(args.ctx, x, y - 1, 6 + ripple * 0.6, 2, "#CBD5E1", 0.12);
  strokeEllipse(args.ctx, x, y, 11.5 + ripple, 3.9, "#E0F2FE", 0.7, 0.1);
  fillEllipse(
    args.ctx,
    x - 7 + wave(frame, 40, 2, 2.2, args.reducedMotion),
    y + 2.4,
    3.4,
    1,
    "#F8FAFC",
    0.08,
  );
}

export function drawLanternGlowPools(
  args: DrawBuddyWorldBaseArgs,
  litCountOverride?: number | null,
): void {
  if (args.world.phase !== "evening" && args.world.phase !== "night") return;
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const frame = safeFrame(args.frame);
  const layerLit = hasWorldLayer(args.world, "lanterns");
  const hasOverride =
    typeof litCountOverride === "number" && Number.isFinite(litCountOverride);
  const litCount = hasOverride
    ? Math.max(0, Math.min(3, Math.floor(litCountOverride)))
    : layerLit
      ? 3
      : 0;
  if (litCount <= 0) return;
  const posts = [
    { x: width * 0.36, y: height * 0.79 },
    { x: width * 0.43, y: height * 0.815 },
    { x: width * 0.5, y: height * 0.84 },
  ];

  for (let index = 0; index < litCount && index < posts.length; index += 1) {
    const post = posts[index];
    const flicker = wave(
      frame,
      26 + index * 4,
      index,
      0.02,
      args.reducedMotion,
    );
    fillEllipse(
      args.ctx,
      post.x + 1,
      post.y + 3,
      13,
      3.6,
      "#FBBF24",
      alphaForMotion(0.07 + flicker, args.reducedMotion),
    );
    if (args.world.phase === "night" && !args.reducedMotion) {
      for (let moth = 0; moth < 2; moth += 1) {
        const angle = frame / (14 + moth * 5) + index * 2.1 + moth * Math.PI;
        fillCircle(
          args.ctx,
          post.x + 1 + Math.cos(angle) * (5 + moth * 2.4),
          post.y - 14 + Math.sin(angle) * (3.4 + moth),
          0.8,
          "#FDE68A",
          0.6,
        );
      }
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
  const hint = worldPaletteHint(args.world);
  const nightish = hint === "night" || hint === "dream";
  const shade = nightish ? "#2A3C52" : "#B5CCDE";
  const crest = nightish ? "#54708E" : "#F2F7FB";

  for (let x = 14; x < width; ) {
    const offset = (x * 13) % 53;
    const px = x + offset;
    const py = baseY + 8 + ((x * 7) % 26);
    const rx = 12 + ((x * 11) % 9);
    fillEllipse(args.ctx, px, py + 1.4, rx, 3, shade, 0.3);
    fillEllipse(args.ctx, px - 2, py, rx * 0.8, 2.2, crest, 0.5);
    x += (args.compact || args.reducedMotion ? 120 : 88) + offset;
  }
}
