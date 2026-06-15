export type BuddyDreamKind =
  | "sky_flight"
  | "giant_acorn"
  | "brick_stars"
  | "snow_drift";

export const BUDDY_DREAM_KINDS: readonly BuddyDreamKind[] = [
  "sky_flight",
  "giant_acorn",
  "brick_stars",
  "snow_drift",
];

export const BUDDY_DREAM_WIDTH = 132;
export const BUDDY_DREAM_HEIGHT = 76;

const UINT_MAX = 4_294_967_295;

function finiteOr(value: number | null | undefined, fallback: number): number {
  return typeof value === "number" && Number.isFinite(value) ? value : fallback;
}

function dreamSeededUnit(seed: number, salt: number): number {
  let value = (finiteOr(seed, 0) + Math.imul(salt + 1, 0x9e3779b9)) >>> 0;
  value ^= value >>> 16;
  value = Math.imul(value, 0x85ebca6b) >>> 0;
  value ^= value >>> 13;
  value = Math.imul(value, 0xc2b2ae35) >>> 0;
  value ^= value >>> 16;
  return (value >>> 0) / UINT_MAX;
}

export function pickBuddyDream(seed: number): BuddyDreamKind {
  const index = Math.floor(dreamSeededUnit(seed, 5) * BUDDY_DREAM_KINDS.length);
  return BUDDY_DREAM_KINDS[Math.min(index, BUDDY_DREAM_KINDS.length - 1)];
}

function fillCircle(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  radius: number,
  color: string,
  alpha: number,
): void {
  ctx.save();
  ctx.globalAlpha = Math.max(0, Math.min(1, alpha));
  ctx.fillStyle = color;
  ctx.beginPath();
  ctx.arc(
    finiteOr(x, 0),
    finiteOr(y, 0),
    Math.max(0, finiteOr(radius, 0)),
    0,
    Math.PI * 2,
  );
  ctx.fill();
  ctx.restore();
}

function fillEllipse(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  radiusX: number,
  radiusY: number,
  color: string,
  alpha: number,
): void {
  ctx.save();
  ctx.globalAlpha = Math.max(0, Math.min(1, alpha));
  ctx.fillStyle = color;
  ctx.beginPath();
  ctx.ellipse(
    finiteOr(x, 0),
    finiteOr(y, 0),
    Math.max(0, finiteOr(radiusX, 0)),
    Math.max(0, finiteOr(radiusY, 0)),
    0,
    0,
    Math.PI * 2,
  );
  ctx.fill();
  ctx.restore();
}

function drawSkyFlight(
  ctx: CanvasRenderingContext2D,
  frame: number,
  width: number,
  height: number,
): void {
  fillEllipse(
    ctx,
    width * 0.25,
    height * 0.95,
    width * 0.4,
    16,
    "#3B4A63",
    0.9,
  );
  fillEllipse(
    ctx,
    width * 0.75,
    height * 1.02,
    width * 0.45,
    20,
    "#2E3A52",
    0.9,
  );
  for (let index = 0; index < 3; index += 1) {
    const cloudX =
      ((frame * (0.18 + index * 0.06) + index * 53) % (width + 40)) - 20;
    fillEllipse(
      ctx,
      cloudX,
      height * (0.22 + index * 0.16),
      13 + index * 3,
      4,
      "#94A3B8",
      0.4,
    );
  }
  const flightX = width * 0.5 + Math.sin(frame / 34) * width * 0.3;
  const flightY = height * 0.4 + Math.cos(frame / 26) * 9;
  fillEllipse(ctx, flightX, flightY, 6, 4.4, "#CBD5E1", 0.95);
  fillCircle(ctx, flightX - 3, flightY - 3.4, 1.8, "#CBD5E1", 0.95);
  fillCircle(ctx, flightX + 3, flightY - 3.4, 1.8, "#CBD5E1", 0.95);
  fillCircle(ctx, flightX - 7, flightY + 1, 1.1, "#E2E8F0", 0.5);
}

function drawGiantAcorn(
  ctx: CanvasRenderingContext2D,
  frame: number,
  width: number,
  height: number,
): void {
  const pulse = 1 + Math.sin(frame / 22) * 0.06;
  const centerX = width / 2;
  const centerY = height * 0.58;
  fillEllipse(ctx, centerX, height * 0.92, 26, 5, "#1E293B", 0.5);
  fillEllipse(
    ctx,
    centerX,
    centerY + 4,
    13 * pulse,
    15 * pulse,
    "#B45309",
    0.95,
  );
  fillEllipse(
    ctx,
    centerX,
    centerY - 9 * pulse,
    14 * pulse,
    6.5 * pulse,
    "#78350F",
    0.95,
  );
  fillCircle(ctx, centerX, centerY - 16 * pulse, 2, "#78350F", 0.95);
  for (let index = 0; index < 4; index += 1) {
    const sparkAngle = frame / 30 + (index * Math.PI) / 2;
    fillCircle(
      ctx,
      centerX + Math.cos(sparkAngle) * 24,
      centerY + Math.sin(sparkAngle) * 14,
      1.2,
      "#FDE68A",
      0.7,
    );
  }
}

function drawBrickStars(
  ctx: CanvasRenderingContext2D,
  frame: number,
  width: number,
  height: number,
): void {
  for (let index = 0; index < 9; index += 1) {
    const starX = (index * 41 + 17) % width;
    const starY = (index * 23 + 9) % height;
    const twinkle = 0.4 + Math.abs(Math.sin(frame / 20 + index)) * 0.5;
    fillCircle(ctx, starX, starY, 0.9, "#E0E7FF", twinkle);
  }
  const swimX = ((frame * 0.7) % (width + 40)) - 20;
  const swimY = height * 0.5 + Math.sin(frame / 16) * 10;
  const tilt = Math.cos(frame / 16) * 0.3;
  fillEllipse(ctx, swimX, swimY, 9, 3.6, "#FB923C", 0.95);
  fillEllipse(ctx, swimX - 9, swimY - tilt * 6, 3.4, 2.2, "#FDBA74", 0.9);
  fillCircle(ctx, swimX + 5.4, swimY - 1, 0.8, "#1E293B", 0.9);
  fillCircle(ctx, swimX - 13, swimY, 0.8, "#FDBA74", 0.4);
}

function drawSnowDrift(
  ctx: CanvasRenderingContext2D,
  frame: number,
  width: number,
  height: number,
): void {
  fillEllipse(
    ctx,
    width * 0.5,
    height * 0.98,
    width * 0.55,
    13,
    "#CBD5E1",
    0.85,
  );
  fillEllipse(
    ctx,
    width * 0.22,
    height * 1.02,
    width * 0.3,
    10,
    "#E2E8F0",
    0.8,
  );
  for (let index = 0; index < 10; index += 1) {
    const flakeX =
      (index * 29 + Math.sin(frame / 30 + index) * 9 + width) % width;
    const flakeY = (frame * (0.35 + (index % 3) * 0.12) + index * 19) % height;
    fillCircle(
      ctx,
      flakeX,
      flakeY,
      index % 3 === 0 ? 1.4 : 0.9,
      "#F8FAFC",
      0.85,
    );
  }
}

export function drawBuddyDreamFrame(
  ctx: CanvasRenderingContext2D,
  kind: BuddyDreamKind,
  frame: number,
  width = BUDDY_DREAM_WIDTH,
  height = BUDDY_DREAM_HEIGHT,
): void {
  const safeFrame = finiteOr(frame, 0);
  const safeWidth = Math.max(1, finiteOr(width, BUDDY_DREAM_WIDTH));
  const safeHeight = Math.max(1, finiteOr(height, BUDDY_DREAM_HEIGHT));
  ctx.clearRect(0, 0, safeWidth, safeHeight);
  ctx.save();
  ctx.globalAlpha = 1;
  ctx.fillStyle = "#1E1B4B";
  ctx.fillRect(0, 0, safeWidth, safeHeight);
  ctx.restore();
  switch (kind) {
    case "sky_flight":
      drawSkyFlight(ctx, safeFrame, safeWidth, safeHeight);
      break;
    case "giant_acorn":
      drawGiantAcorn(ctx, safeFrame, safeWidth, safeHeight);
      break;
    case "brick_stars":
      drawBrickStars(ctx, safeFrame, safeWidth, safeHeight);
      break;
    case "snow_drift":
      drawSnowDrift(ctx, safeFrame, safeWidth, safeHeight);
      break;
  }
}
