import { describe, expect, it, vi } from "vitest";
import { drawBuddyWorld } from "../features/Buddy/buddyWorldDraw";
import {
  drawAmbientLayers,
  drawObservatoryStructures,
  drawStarField,
  shouldDrawStarField,
} from "../features/Buddy/buddyWorldDrawAtmosphere";
import { drawLanterns } from "../features/Buddy/buddyWorldDrawDiorama";
import {
  buildBuddyWorldState,
  type BuddyWorldState,
} from "../features/Buddy/buddyWorldModel";
import { PALETTES } from "../features/Buddy/constants";
import type { BuddyPetState, BuddyPulse } from "../features/Buddy/types";

type MockCanvasContext = Pick<
  CanvasRenderingContext2D,
  | "arc"
  | "beginPath"
  | "bezierCurveTo"
  | "clearRect"
  | "closePath"
  | "createLinearGradient"
  | "ellipse"
  | "fill"
  | "fillRect"
  | "fillText"
  | "lineTo"
  | "moveTo"
  | "restore"
  | "save"
  | "stroke"
  | "translate"
> &
  Partial<CanvasRenderingContext2D>;

type RecordedCanvasContext = CanvasRenderingContext2D & {
  alphaWrites: number[];
  compositeWrites: string[];
  drawOps: string[];
  fillRectStyles: string[];
};

interface CanvasDrawOp {
  x: number;
  y: number;
  width: number;
  height: number;
  color: string;
  alpha: number;
}

interface StarFieldSignature {
  matchedCount: number;
  duplicateCount: number;
}

interface FullCanvasOverlay {
  color: string;
  alpha: number;
}

interface DrawBranchCase {
  label: string;
  world: BuddyWorldState;
  expectedStyles: string[];
}

function makeCanvasContext(): RecordedCanvasContext {
  const gradientStops: string[] = [];
  const gradient = {
    addColorStop: vi.fn((offset: number, color: string) => {
      gradientStops.push(`stop:${offset.toFixed(3)}:${color}`);
    }),
  } as unknown as CanvasGradient;
  const alphaWrites: number[] = [];
  const compositeWrites: string[] = [];
  const drawOps: string[] = [];
  const fillRectStyles: string[] = [];
  let globalAlphaValue = 1;
  let globalCompositeOperationValue: GlobalCompositeOperation = "source-over";
  let fillStyleValue: CanvasRenderingContext2D["fillStyle"] = "#000000";
  let strokeStyleValue: CanvasRenderingContext2D["strokeStyle"] = "#000000";
  const formatNumber = (value: number) => value.toFixed(3);
  const ctx: MockCanvasContext & {
    alphaWrites: number[];
    compositeWrites: string[];
    drawOps: string[];
    fillRectStyles: string[];
  } = {
    alphaWrites,
    compositeWrites,
    drawOps,
    fillRectStyles,
    arc: vi.fn(
      (
        x: number,
        y: number,
        radius: number,
        startAngle: number,
        endAngle: number,
      ) => {
        drawOps.push(
          `arc:${formatNumber(x)}:${formatNumber(y)}:${formatNumber(
            radius,
          )}:${formatNumber(startAngle)}:${formatNumber(endAngle)}`,
        );
      },
    ),
    beginPath: vi.fn(() => drawOps.push("beginPath")),
    bezierCurveTo: vi.fn(
      (
        cp1x: number,
        cp1y: number,
        cp2x: number,
        cp2y: number,
        x: number,
        y: number,
      ) => {
        drawOps.push(
          `bezierCurveTo:${formatNumber(cp1x)}:${formatNumber(
            cp1y,
          )}:${formatNumber(cp2x)}:${formatNumber(cp2y)}:${formatNumber(
            x,
          )}:${formatNumber(y)}`,
        );
      },
    ),
    clearRect: vi.fn((x: number, y: number, width: number, height: number) => {
      drawOps.push(
        `clearRect:${formatNumber(x)}:${formatNumber(y)}:${formatNumber(
          width,
        )}:${formatNumber(height)}`,
      );
    }),
    closePath: vi.fn(() => drawOps.push("closePath")),
    createLinearGradient: vi.fn(
      (x0: number, y0: number, x1: number, y1: number) => {
        drawOps.push(
          `createLinearGradient:${formatNumber(x0)}:${formatNumber(
            y0,
          )}:${formatNumber(x1)}:${formatNumber(y1)}`,
        );
        return gradient;
      },
    ),
    ellipse: vi.fn(
      (
        x: number,
        y: number,
        radiusX: number,
        radiusY: number,
        rotation: number,
        startAngle: number,
        endAngle: number,
      ) => {
        drawOps.push(
          `ellipse:${formatNumber(x)}:${formatNumber(y)}:${formatNumber(
            radiusX,
          )}:${formatNumber(radiusY)}:${formatNumber(rotation)}:${formatNumber(
            startAngle,
          )}:${formatNumber(endAngle)}`,
        );
      },
    ),
    fill: vi.fn(() =>
      drawOps.push(`fill:${String(fillStyleValue)}:${globalAlphaValue}`),
    ),
    fillRect: vi.fn((x: number, y: number, width: number, height: number) => {
      fillRectStyles.push(String(fillStyleValue));
      drawOps.push(
        `fillRect:${formatNumber(x)}:${formatNumber(y)}:${formatNumber(
          width,
        )}:${formatNumber(height)}:${String(
          fillStyleValue,
        )}:${globalAlphaValue}`,
      );
    }),
    fillText: vi.fn((text: string, x: number, y: number) => {
      drawOps.push(
        `fillText:${text}:${formatNumber(x)}:${formatNumber(y)}:${String(
          fillStyleValue,
        )}:${globalAlphaValue}`,
      );
    }),
    lineTo: vi.fn((x: number, y: number) => {
      drawOps.push(`lineTo:${formatNumber(x)}:${formatNumber(y)}`);
    }),
    moveTo: vi.fn((x: number, y: number) => {
      drawOps.push(`moveTo:${formatNumber(x)}:${formatNumber(y)}`);
    }),
    restore: vi.fn(() => drawOps.push("restore")),
    save: vi.fn(() => drawOps.push("save")),
    stroke: vi.fn(() =>
      drawOps.push(`stroke:${String(strokeStyleValue)}:${globalAlphaValue}`),
    ),
    translate: vi.fn((x: number, y: number) => {
      drawOps.push(`translate:${formatNumber(x)}:${formatNumber(y)}`);
    }),
    font: "10px monospace",
    get fillStyle() {
      return fillStyleValue;
    },
    set fillStyle(value: CanvasRenderingContext2D["fillStyle"]) {
      fillStyleValue = value === gradient ? gradientStops.join("|") : value;
    },
    get globalAlpha() {
      return globalAlphaValue;
    },
    set globalAlpha(value: number) {
      alphaWrites.push(value);
      globalAlphaValue = value;
    },
    get globalCompositeOperation() {
      return globalCompositeOperationValue;
    },
    set globalCompositeOperation(value: GlobalCompositeOperation) {
      compositeWrites.push(value);
      globalCompositeOperationValue = value;
    },
    imageSmoothingEnabled: false,
    lineCap: "round" as CanvasLineCap,
    lineWidth: 1,
    get strokeStyle() {
      return strokeStyleValue;
    },
    set strokeStyle(value: CanvasRenderingContext2D["strokeStyle"]) {
      strokeStyleValue = value;
    },
    textAlign: "center" as CanvasTextAlign,
    textBaseline: "middle" as CanvasTextBaseline,
  };
  return ctx as RecordedCanvasContext;
}

function makePet(overrides?: Partial<BuddyPetState>): BuddyPetState {
  return {
    needs: {
      hunger: 80,
      energy: 80,
      hygiene: 80,
      boredom: 10,
      affection: 80,
    },
    condition: {
      sleeping: false,
      hungry: false,
      sleepy: false,
      dirty: false,
      bored: false,
      lonely: false,
    },
    evolution: {
      care_score: 0,
      neglect_score: 0,
      open_seconds: 0,
      last_evolved_at: null,
    },
    ...overrides,
  };
}

function makePulse(overrides?: Partial<BuddyPulse>): BuddyPulse {
  const pulse: BuddyPulse = {
    generated_at: "2024-01-01T00:00:00Z",
    tasks: { total: 3, stuck: 0, abandoned: 0, by_status: {} },
    trajectories: { total: 10, untitled: 0, oldest_age_days: 1 },
    memory: { total: 5, orphan: 0, stale_conflicts: 0 },
    providers: { defaults_ok: true, broken_refs: 0, quota_warnings: 0 },
    mcp: { total: 4, failing: 0, auth_expiring: 0 },
    customization: { modes: 3, skills: 2, commands: 1, subagents: 0, hooks: 0 },
    diagnostics: { last_hour: 0, top_error_types: [] },
    git: { uncommitted_files: 0, diff_lines_4h: 0, branches: 3 },
    worktrees: {
      total_registered: 3,
      total_discovered: 1,
      total: 4,
      clean: 2,
      dirty: 1,
      unknown: 0,
      stale: 1,
      conflicted: 0,
      shared: 1,
      abandoned_clean: 2,
      changed_files: 3,
      additions: 10,
      deletions: 2,
      missing_registry_paths: 1,
      unregistered_cache_dirs: 1,
      merged_branches: 2,
    },
  };
  return { ...pulse, ...overrides };
}

function makeWorld(args?: {
  now?: Date;
  pulse?: BuddyPulse | null;
  pet?: BuddyPetState;
}): BuddyWorldState {
  return buildBuddyWorldState({
    now: args?.now ?? new Date("2024-01-01T14:00:00"),
    pulse: args?.pulse ?? makePulse(),
    pet: args?.pet ?? makePet(),
    nowPlaying: null,
    activeQuest: null,
  });
}

function drawWorld(
  world: BuddyWorldState,
  ctx = makeCanvasContext(),
): RecordedCanvasContext {
  drawBuddyWorld({
    ctx,
    world,
    palette: PALETTES[0],
    frame: 120,
    width: 720,
    height: 260,
    compact: false,
    reducedMotion: false,
  });
  return ctx;
}

function drawWorldWithOptions(
  world: BuddyWorldState,
  options?: Partial<{
    compact: boolean;
    reducedMotion: boolean;
    frame: number;
    width: number;
    height: number;
  }>,
): RecordedCanvasContext {
  const ctx = makeCanvasContext();
  drawBuddyWorld({
    ctx,
    world,
    palette: PALETTES[0],
    frame: options?.frame ?? 120,
    width: options?.width ?? 720,
    height: options?.height ?? 260,
    compact: options?.compact ?? false,
    reducedMotion: options?.reducedMotion ?? false,
  });
  return ctx;
}

const starFieldArgs = {
  palette: PALETTES[0],
  frame: 120,
  width: 720,
  height: 260,
  compact: false,
  reducedMotion: false,
};

function parseFillRectOperation(operation: string): CanvasDrawOp | null {
  const parts = operation.split(":");
  if (parts[0] !== "fillRect") return null;
  const alpha = Number(parts.at(-1));
  return {
    x: Number(parts[1]),
    y: Number(parts[2]),
    width: Number(parts[3]),
    height: Number(parts[4]),
    color: parts.slice(5, -1).join(":"),
    alpha,
  };
}

function fillRectOperationKey(operation: CanvasDrawOp): string {
  return `fillRect:${operation.x.toFixed(3)}:${operation.y.toFixed(
    3,
  )}:${operation.width.toFixed(3)}:${operation.height.toFixed(3)}:${
    operation.color
  }:${operation.alpha}`;
}

function starFieldOperationSignature(
  ctx: RecordedCanvasContext,
  starOnlyCtx: RecordedCanvasContext,
): StarFieldSignature {
  const expected = starOnlyCtx.drawOps
    .map(parseFillRectOperation)
    .filter((operation): operation is CanvasDrawOp => operation !== null)
    .filter(
      (operation) =>
        operation.color === "#FFFFFF" || operation.color === "#FDE68A",
    );
  const expectedCounts = new Map<string, number>();
  const fullCounts = new Map<string, number>();

  for (const operation of expected) {
    const key = fillRectOperationKey(operation);
    expectedCounts.set(key, (expectedCounts.get(key) ?? 0) + 1);
  }

  for (const operation of ctx.drawOps) {
    const parsed = parseFillRectOperation(operation);
    if (!parsed) continue;
    const key = fillRectOperationKey(parsed);
    fullCounts.set(key, (fullCounts.get(key) ?? 0) + 1);
  }

  let matchedCount = 0;
  let duplicateCount = 0;
  for (const [key, expectedCount] of expectedCounts.entries()) {
    const actualCount = fullCounts.get(key) ?? 0;
    matchedCount += Math.min(actualCount, expectedCount);
    duplicateCount += Math.max(0, actualCount - expectedCount);
  }

  return { matchedCount, duplicateCount };
}

function expectAlphaWritesClamped(ctx: RecordedCanvasContext): void {
  expect(ctx.alphaWrites.length).toBeGreaterThan(0);
  expect(ctx.alphaWrites.every((alpha) => alpha >= 0 && alpha <= 1)).toBe(true);
}

function expectDrawOpsFinite(ctx: RecordedCanvasContext): void {
  const serializedOps = ctx.drawOps.join(":");
  const numericTokens = ctx.drawOps.flatMap((operation) =>
    operation
      .split(":")
      .map((token) => Number(token))
      .filter((value) => !Number.isNaN(value)),
  );
  expect(serializedOps).not.toMatch(/\b(?:NaN|Infinity|-Infinity)\b/);
  expect(numericTokens.length).toBeGreaterThan(0);
  expect(numericTokens.every((value) => Number.isFinite(value))).toBe(true);
}

function expectHealthyDraw(ctx: RecordedCanvasContext): void {
  expectAlphaWritesClamped(ctx);
  expectDrawOpsFinite(ctx);
}

function expectFillStyles(ctx: RecordedCanvasContext, styles: string[]): void {
  for (const style of styles) {
    expect(ctx.fillRectStyles).toContain(style);
  }
}

function fillRectStyleCount(ctx: RecordedCanvasContext, style: string): number {
  return ctx.fillRectStyles.filter((item) => item === style).length;
}

function strokeStyleCount(ctx: RecordedCanvasContext, style: string): number {
  return ctx.drawOps.filter((operation) =>
    operation.startsWith(`stroke:${style}:`),
  ).length;
}

function fullCanvasOverlays(ctx: RecordedCanvasContext): FullCanvasOverlay[] {
  return ctx.drawOps
    .map(parseFillRectOperation)
    .filter((operation): operation is CanvasDrawOp => operation !== null)
    .filter(
      (operation) =>
        operation.x === 0 &&
        operation.y === 0 &&
        operation.width === 720 &&
        operation.height === 260,
    )
    .filter((operation) => !operation.color.includes("stop:"))
    .map((operation) => ({
      color: operation.color,
      alpha: operation.alpha,
    }));
}

function skyGradientOperations(ctx: RecordedCanvasContext): string[] {
  return ctx.drawOps.filter(
    (operation) =>
      operation.startsWith("clearRect:") ||
      operation.startsWith("createLinearGradient:") ||
      operation.includes("stop:"),
  );
}

interface PaletteCase {
  label: string;
  now: Date;
  pet?: BuddyPetState;
  pulse?: BuddyPulse;
}

describe("drawBuddyWorld", () => {
  it("resets the canvas and draws one deterministic sky gradient per frame", () => {
    const world = makeWorld({ now: new Date("2024-01-01T23:00:00") });
    const firstCtx = drawWorldWithOptions(world, { frame: 120 });
    const secondCtx = drawWorldWithOptions(world, { frame: 120 });
    const nextFrameCtx = drawWorldWithOptions(world, { frame: 121 });

    expect(firstCtx.drawOps[0]).toBe("clearRect:0.000:0.000:720.000:260.000");
    expect(firstCtx.compositeWrites).toContain("source-over");
    expect(firstCtx.alphaWrites[0]).toBe(1);
    expect(firstCtx.alphaWrites[1]).toBe(1);
    expect(
      firstCtx.drawOps.filter((operation) =>
        operation.startsWith("createLinearGradient:"),
      ),
    ).toHaveLength(1);
    expect(
      firstCtx.fillRectStyles.filter((style) => style.includes("stop:")),
    ).toHaveLength(1);
    expect(skyGradientOperations(secondCtx)).toEqual(
      skyGradientOperations(firstCtx),
    );
    expect(skyGradientOperations(nextFrameCtx)).toEqual(
      skyGradientOperations(firstCtx),
    );
  });

  it.each<PaletteCase>([
    { label: "morning", now: new Date("2024-01-01T08:00:00") },
    { label: "day", now: new Date("2024-01-01T14:00:00") },
    { label: "evening", now: new Date("2024-01-01T18:00:00") },
    { label: "night", now: new Date("2024-01-01T23:00:00") },
    {
      label: "dream",
      now: new Date("2024-01-01T23:00:00"),
      pet: makePet({ condition: { ...makePet().condition, sleeping: true } }),
    },
    {
      label: "storm",
      now: new Date("2024-01-01T14:00:00"),
      pet: makePet(),
      pulse: makePulse({
        providers: { defaults_ok: true, broken_refs: 1, quota_warnings: 0 },
      }),
    },
  ])(
    "draws the $label palette hint without throwing",
    ({ now, pet, pulse }) => {
      const world = makeWorld({ now, pet, pulse });
      const ctx = drawWorld(world);

      expectHealthyDraw(ctx);
    },
  );

  it.each<DrawBranchCase>([
    {
      label: "morning",
      world: makeWorld({ now: new Date("2024-01-01T08:00:00") }),
      expectedStyles: ["#FDE68A", "#86EFAC"],
    },
    {
      label: "day",
      world: makeWorld({ now: new Date("2024-01-01T14:00:00") }),
      expectedStyles: ["#FBBF24", "#BBF7D0"],
    },
    {
      label: "evening",
      world: makeWorld({ now: new Date("2024-01-01T18:00:00") }),
      expectedStyles: ["#FB923C", "#FDBA74", "#F9A8D4"],
    },
    {
      label: "night",
      world: makeWorld({ now: new Date("2024-01-01T23:00:00") }),
      expectedStyles: ["#E0E7FF", "#FFFFFF", "#A7F3D0"],
    },
  ])(
    "executes distinct $label visual branches",
    ({ world, expectedStyles }) => {
      const ctx = drawWorld(world);

      expectFillStyles(ctx, expectedStyles);
      expectHealthyDraw(ctx);
    },
  );

  it.each<DrawBranchCase>([
    {
      label: "dream mist",
      world: makeWorld({
        pet: makePet({ condition: { ...makePet().condition, sleeping: true } }),
      }),
      expectedStyles: ["#C4B5FD"],
    },
    {
      label: "empty food nook",
      world: makeWorld({
        pet: makePet({ condition: { ...makePet().condition, hungry: true } }),
      }),
      expectedStyles: ["#92400E", "#FDE68A"],
    },
    {
      label: "toy glow",
      world: makeWorld({
        pet: makePet({ condition: { ...makePet().condition, bored: true } }),
      }),
      expectedStyles: ["#F9A8D4", "#A78BFA"],
    },
    {
      label: "cozy home glow",
      world: makeWorld({
        pet: makePet({ needs: { ...makePet().needs, affection: 90 } }),
      }),
      expectedStyles: ["#F9A8D4", "#FCA5A5"],
    },
  ])(
    "draws care layer $label without invalid values",
    ({ world, expectedStyles }) => {
      const ctx = drawWorld(world);

      expectFillStyles(ctx, expectedStyles);
      expectHealthyDraw(ctx);
    },
  );

  it("draws active runtime workshop runes and work energy", () => {
    const world = buildBuddyWorldState({
      now: new Date("2024-01-01T14:00:00"),
      pulse: makePulse({ diagnostics: { last_hour: 0, top_error_types: [] } }),
      pet: makePet(),
      nowPlaying: {
        id: "runtime-active-draw",
        signal_type: "tool_used",
        title: "Running tests",
        source: "test",
        status: "progress",
        priority: "normal",
        created_at: "2024-01-01T14:00:00Z",
        persistent: true,
      },
      activeQuest: null,
    });
    const ctx = drawWorld(world);

    expect(world.atmosphere.layers).toContain("workshop_runes");
    expectFillStyles(ctx, ["#67E8F9", "#60A5FA"]);
    expect(strokeStyleCount(ctx, "#38BDF8")).toBeGreaterThan(0);
    expect(strokeStyleCount(ctx, "#A78BFA")).toBeGreaterThan(0);
    expectHealthyDraw(ctx);
  });

  it("keeps provider warning distinct from provider storm", () => {
    const warningWorld = makeWorld({
      pulse: makePulse({
        providers: { defaults_ok: false, broken_refs: 0, quota_warnings: 2 },
        diagnostics: { last_hour: 0, top_error_types: [] },
      }),
    });
    const criticalWorld = makeWorld({
      pulse: makePulse({
        providers: { defaults_ok: true, broken_refs: 2, quota_warnings: 0 },
      }),
    });
    const warningCtx = drawWorld(warningWorld);
    const criticalCtx = drawWorld(criticalWorld);

    expect(warningWorld.atmosphere.layers).toContain("provider_flicker");
    expect(warningWorld.atmosphere.layers).not.toContain("provider_storm");
    expect(criticalWorld.atmosphere.layers).toContain("provider_storm");
    expect(fillRectStyleCount(warningCtx, "#020617")).toBeLessThan(
      fillRectStyleCount(criticalCtx, "#020617"),
    );
    expect(fillRectStyleCount(warningCtx, "#FACC15")).toBeLessThan(
      fillRectStyleCount(criticalCtx, "#FACC15"),
    );
    expectFillStyles(warningCtx, ["#F59E0B"]);
    expectFillStyles(criticalCtx, ["#F87171", "#FACC15"]);
    expectHealthyDraw(warningCtx);
    expectHealthyDraw(criticalCtx);
  });

  it("draws memory attention orbs and active memory streams", () => {
    const attentionWorld = makeWorld({
      pulse: makePulse({
        memory: { total: 12, orphan: 4, stale_conflicts: 0 },
      }),
    });
    const activeWorld = buildBuddyWorldState({
      now: new Date("2024-01-01T14:00:00"),
      pulse: makePulse({
        memory: { total: 12, orphan: 0, stale_conflicts: 0 },
      }),
      pet: makePet(),
      nowPlaying: {
        id: "memory-runtime-draw",
        signal_type: "memory_extract",
        title: "Extracting memories",
        source: "memory",
        status: "progress",
        priority: "normal",
        created_at: "2024-01-01T14:00:00Z",
        persistent: true,
      },
      activeQuest: null,
    });
    const attentionCtx = drawWorld(attentionWorld);
    const activeCtx = drawWorld(activeWorld);

    expect(attentionWorld.atmosphere.layers).toContain("memory_orbs");
    expect(activeWorld.atmosphere.layers).toContain("memory_orbs");
    expectFillStyles(attentionCtx, ["#FBBF24", "#FDE68A"]);
    expectFillStyles(activeCtx, ["#FBBF24", "#FDE68A"]);
    expect(strokeStyleCount(activeCtx, "#FDE68A")).toBeGreaterThan(
      strokeStyleCount(attentionCtx, "#FDE68A"),
    );
    expectHealthyDraw(attentionCtx);
    expectHealthyDraw(activeCtx);
  });

  it("uses lower bounded effect counts for compact reduced-motion paths", () => {
    const world = makeWorld({ now: new Date("2024-01-01T23:00:00") });
    const standardCtx = drawWorldWithOptions(world, {
      compact: false,
      reducedMotion: false,
    });
    const reducedCtx = drawWorldWithOptions(world, {
      compact: true,
      reducedMotion: true,
      width: 360,
      height: 190,
    });

    expect(fillRectStyleCount(reducedCtx, "#FFFFFF")).toBeLessThan(
      fillRectStyleCount(standardCtx, "#FFFFFF"),
    );
    expect(reducedCtx.drawOps.length).toBeLessThan(standardCtx.drawOps.length);
    expectHealthyDraw(standardCtx);
    expectHealthyDraw(reducedCtx);
  });

  it("draws all supported atmosphere layers without throwing", () => {
    const baseWorld = makeWorld();
    const world: BuddyWorldState = {
      ...baseWorld,
      weather: "aurora",
      atmosphere: {
        phase: baseWorld.phase,
        mood: "busy",
        primaryWeather: "aurora",
        layers: [
          "sun_motes",
          "moths",
          "fireflies",
          "stars",
          "aurora",
          "dream_mist",
          "workshop_runes",
          "provider_storm",
          "provider_flicker",
          "memory_orbs",
          "cozy_home_glow",
          "toy_glow",
          "empty_food_nook",
        ],
        intensity: 0.86,
        paletteHint: "storm",
        serious: true,
      },
    };
    const ctx = drawWorld(world);

    expectHealthyDraw(ctx);
  });

  it("draws compact reduced-motion mode without throwing", () => {
    const ctx = makeCanvasContext();

    drawBuddyWorld({
      ctx,
      world: makeWorld({ now: new Date("2024-01-01T23:00:00") }),
      palette: PALETTES[0],
      frame: 4,
      width: 360,
      height: 190,
      compact: true,
      reducedMotion: true,
    });

    expectHealthyDraw(ctx);
  });

  it("keeps storm, dream mist, memory orbs, and workshop runes finite with edge inputs", () => {
    const baseWorld = makeWorld({
      pulse: makePulse({
        providers: { defaults_ok: false, broken_refs: 2, quota_warnings: 3 },
      }),
    });
    const world: BuddyWorldState = {
      ...baseWorld,
      celestialX: Number.POSITIVE_INFINITY,
      celestialY: Number.NaN,
      weatherX: Number.NEGATIVE_INFINITY,
      weatherY: Number.NaN,
      atmosphere: {
        ...baseWorld.atmosphere,
        layers: [
          "provider_storm",
          "dream_mist",
          "memory_orbs",
          "workshop_runes",
        ],
        intensity: Number.POSITIVE_INFINITY,
        paletteHint: "storm",
      },
      objects: baseWorld.objects.map((item, index) => ({
        ...item,
        x: index % 2 === 0 ? Number.NaN : item.x,
        y: index % 2 === 1 ? Number.POSITIVE_INFINITY : item.y,
        size: Number.POSITIVE_INFINITY,
        intensity: Number.NaN,
        interactionX: Number.NEGATIVE_INFINITY,
        interactionY: Number.NaN,
        depthScale: Number.POSITIVE_INFINITY,
      })),
    };
    const ctx = makeCanvasContext();

    drawBuddyWorld({
      ctx,
      world,
      palette: PALETTES[0],
      frame: Number.POSITIVE_INFINITY,
      width: Number.POSITIVE_INFINITY,
      height: Number.NaN,
      compact: false,
      reducedMotion: false,
    });

    expectHealthyDraw(ctx);
  });

  it("draw output is deterministic for the same world and frame", () => {
    const world = makeWorld({
      now: new Date("2024-01-01T18:00:00"),
      pulse: makePulse({
        memory: { total: 12, orphan: 3, stale_conflicts: 1 },
        providers: { defaults_ok: false, broken_refs: 0, quota_warnings: 2 },
        diagnostics: { last_hour: 8, top_error_types: ["tool_failed"] },
      }),
    });
    const firstCtx = makeCanvasContext();
    const secondCtx = makeCanvasContext();
    const args = {
      world,
      palette: PALETTES[0],
      frame: 240,
      width: 720,
      height: 260,
      compact: false,
      reducedMotion: false,
    };

    drawBuddyWorld({ ctx: firstCtx, ...args });
    drawBuddyWorld({ ctx: secondCtx, ...args });

    expect(secondCtx.drawOps).toEqual(firstCtx.drawOps);
  });

  it("does not draw the star field without the stars layer", () => {
    const world = makeWorld({ now: new Date("2024-01-01T14:00:00") });
    const ctx = drawWorld(world);
    const starOnlyCtx = makeCanvasContext();

    drawStarField({ ctx: starOnlyCtx, world, ...starFieldArgs });

    const signature = starFieldOperationSignature(ctx, starOnlyCtx);

    expect(world.atmosphere.layers).not.toContain("stars");
    expect(shouldDrawStarField(world)).toBe(false);
    expect(starOnlyCtx.fillRectStyles).toHaveLength(54);
    expect(signature.matchedCount).toBe(0);
    expect(signature.duplicateCount).toBe(0);
    expectHealthyDraw(ctx);
    expectHealthyDraw(starOnlyCtx);
  });

  it("draws one bounded star-field pass for the stars layer", () => {
    const world = makeWorld({ now: new Date("2024-01-01T23:00:00") });
    const ctx = drawWorld(world);
    const starOnlyCtx = makeCanvasContext();
    const structuresOnlyCtx = makeCanvasContext();

    drawStarField({ ctx: starOnlyCtx, world, ...starFieldArgs });
    drawObservatoryStructures({
      ctx: structuresOnlyCtx,
      world,
      ...starFieldArgs,
    });

    const signature = starFieldOperationSignature(ctx, starOnlyCtx);
    const structureSignature = starFieldOperationSignature(
      structuresOnlyCtx,
      starOnlyCtx,
    );

    expect(world.atmosphere.layers).toContain("stars");
    expect(shouldDrawStarField(world)).toBe(true);
    expect(starOnlyCtx.fillRectStyles).toHaveLength(54);
    expect(signature.matchedCount).toBe(54);
    expect(signature.duplicateCount).toBe(0);
    expect(structureSignature.matchedCount).toBe(0);
    expect(structureSignature.duplicateCount).toBe(0);
    expectHealthyDraw(ctx);
    expectHealthyDraw(starOnlyCtx);
    expectHealthyDraw(structuresOnlyCtx);
  });

  it("draws one bounded aurora pass from the atmosphere layer", () => {
    const world = makeWorld({ now: new Date("2024-01-01T23:00:00") });
    const ctx = drawWorld(world);

    expect(world.weather).toBe("aurora");
    expect(world.atmosphere.layers).toContain("aurora");
    expect(strokeStyleCount(ctx, "#2DD4BF")).toBe(2);
    expect(strokeStyleCount(ctx, "#A855F7")).toBe(1);
    expectHealthyDraw(ctx);
  });

  it("does not emit day sun motes for a night world without the sun_motes layer", () => {
    const world = makeWorld({ now: new Date("2024-01-01T23:00:00") });
    const ctx = drawWorld(world);
    const sunMoteWorld: BuddyWorldState = {
      ...world,
      atmosphere: {
        ...world.atmosphere,
        layers: ["sun_motes"],
      },
    };
    const sunMotesOnlyCtx = makeCanvasContext();

    drawAmbientLayers({
      ctx: sunMotesOnlyCtx,
      world: sunMoteWorld,
      ...starFieldArgs,
    });

    expect(world.phase).toBe("night");
    expect(world.atmosphere.layers).not.toContain("sun_motes");
    expect(world.celestialLabel).toBe("Moon");
    expect(fillRectStyleCount(ctx, "#E0E7FF")).toBeGreaterThan(0);
    expect(fillRectStyleCount(ctx, "#F59E0B")).toBe(0);
    expect(fillRectStyleCount(sunMotesOnlyCtx, "#FDE68A")).toBe(0);
    expectHealthyDraw(ctx);
  });

  it("keeps calm night, aurora, and provider warning worlds free of extra high-alpha full-canvas overlays", () => {
    const calmNight = makeWorld({ now: new Date("2024-01-01T23:00:00") });
    const providerWarning = makeWorld({
      now: new Date("2024-01-01T23:00:00"),
      pulse: makePulse({
        providers: { defaults_ok: false, broken_refs: 0, quota_warnings: 2 },
        diagnostics: { last_hour: 0, top_error_types: [] },
      }),
    });

    for (const world of [calmNight, providerWarning]) {
      const overlays = fullCanvasOverlays(drawWorld(world));

      expect(world.atmosphere.serious).toBe(false);
      expect(overlays).toHaveLength(0);
    }
  });

  it("draws one clamped full-canvas storm overlay for provider critical worlds", () => {
    const world = makeWorld({
      pulse: makePulse({
        providers: { defaults_ok: true, broken_refs: 2, quota_warnings: 0 },
      }),
    });
    const ctx = drawWorld(world);
    const overlays = fullCanvasOverlays(ctx);

    expect(world.atmosphere.layers).toContain("provider_storm");
    expect(overlays).toHaveLength(1);
    expect(overlays[0]?.color).toBe("#020617");
    expect(overlays[0]?.alpha).toBeCloseTo(0.1936);
    expect(
      overlays.every((overlay) => overlay.alpha >= 0 && overlay.alpha <= 1),
    ).toBe(true);
    expect(fillRectStyleCount(ctx, "#FACC15")).toBeGreaterThanOrEqual(2);
    expectHealthyDraw(ctx);
  });
  it("emits parallax translate bands for standard motion only", () => {
    const world = makeWorld({ now: new Date("2024-01-01T23:00:00") });
    const standardCtx = drawWorldWithOptions(world, { frame: 240 });
    const reducedCtx = drawWorldWithOptions(world, {
      frame: 240,
      reducedMotion: true,
    });
    const translates = standardCtx.drawOps.filter((operation) =>
      operation.startsWith("translate:"),
    );

    expect(translates.length).toBeGreaterThan(0);
    expect(
      reducedCtx.drawOps.some((operation) =>
        operation.startsWith("translate:"),
      ),
    ).toBe(false);
    expectHealthyDraw(standardCtx);
    expectHealthyDraw(reducedCtx);
  });

  it("draws journey dust while the actor travels and accents after arrival", () => {
    const world = makeWorld();
    const baseCtx = drawWorld(world);
    const travelingCtx = makeCanvasContext();
    const arrivedCtx = makeCanvasContext();
    const baseArgs = {
      world,
      palette: PALETTES[0],
      frame: 120,
      width: 720,
      height: 260,
      compact: false,
      reducedMotion: false,
    };

    drawBuddyWorld({
      ctx: travelingCtx,
      ...baseArgs,
      actor: {
        xPercent: 58,
        yPercent: 81,
        intentKind: "warm_by_fire",
        travel: {
          fromXPercent: 33,
          fromYPercent: 76,
          startedAtMs: 0,
          durationMs: 3_800,
        },
        nowMs: 1_900,
      },
    });
    drawBuddyWorld({
      ctx: arrivedCtx,
      ...baseArgs,
      actor: {
        xPercent: 36,
        yPercent: 82,
        intentKind: "visit_pond",
        travel: null,
        nowMs: 9_000,
      },
    });

    const dustOps = travelingCtx.drawOps.filter((operation) =>
      operation.startsWith("fill:#D6CDBF"),
    );
    const rippleOps = arrivedCtx.drawOps.filter((operation) =>
      operation.startsWith("stroke:#7DD3FC"),
    );

    expect(dustOps.length).toBeGreaterThan(0);
    expect(rippleOps.length).toBeGreaterThan(0);
    expect(
      baseCtx.drawOps.some((operation) => operation.startsWith("fill:#D6CDBF")),
    ).toBe(false);
    expectHealthyDraw(travelingCtx);
    expectHealthyDraw(arrivedCtx);
  });

  it("keeps actor drawing finite with hostile travel inputs", () => {
    const world = makeWorld();
    const ctx = makeCanvasContext();

    drawBuddyWorld({
      ctx,
      world,
      palette: PALETTES[0],
      frame: Number.NaN,
      width: 720,
      height: 260,
      compact: false,
      reducedMotion: false,
      actor: {
        xPercent: Number.POSITIVE_INFINITY,
        yPercent: Number.NaN,
        intentKind: "splash_puddles",
        travel: {
          fromXPercent: Number.NEGATIVE_INFINITY,
          fromYPercent: Number.NaN,
          startedAtMs: Number.NaN,
          durationMs: 0,
        },
        nowMs: Number.POSITIVE_INFINITY,
      },
    });

    expectHealthyDraw(ctx);
  });

  function drawActorWorld(args: {
    intentKind: string;
    heldMs: number;
    xPercent?: number;
    yPercent?: number;
  }): RecordedCanvasContext {
    const ctx = makeCanvasContext();
    drawBuddyWorld({
      ctx,
      world: makeWorld(),
      palette: PALETTES[0],
      frame: 120,
      width: 720,
      height: 260,
      compact: false,
      reducedMotion: false,
      actor: {
        xPercent: args.xPercent ?? 47,
        yPercent: args.yPercent ?? 80,
        intentKind: args.intentKind,
        travel: null,
        nowMs: 50_000,
        intentStartedAtMs: 50_000 - args.heldMs,
      },
    });
    return ctx;
  }

  it("draws progressive feed care props that finish with a heart", () => {
    const earlyCtx = drawActorWorld({
      intentKind: "care_feed",
      heldMs: 400,
      xPercent: 38,
      yPercent: 78,
    });
    const lateCtx = drawActorWorld({
      intentKind: "care_feed",
      heldMs: 7_500,
      xPercent: 38,
      yPercent: 78,
    });

    const foodCount = (ctx: RecordedCanvasContext) =>
      ctx.drawOps.filter((operation) => operation.startsWith("fill:#FDBA74"))
        .length;
    const bowlCount = (ctx: RecordedCanvasContext) =>
      ctx.drawOps.filter((operation) => operation.startsWith("fill:#92400E"))
        .length;

    expect(foodCount(earlyCtx)).toBeGreaterThan(foodCount(lateCtx));
    expect(bowlCount(earlyCtx)).toBeGreaterThan(0);
    expect(bowlCount(lateCtx)).toBeGreaterThan(0);
    expect(
      earlyCtx.drawOps.some(
        (operation) =>
          operation.startsWith("fillText:♥") && operation.includes("#F472B6"),
      ),
    ).toBe(false);
    expect(
      lateCtx.drawOps.some(
        (operation) =>
          operation.startsWith("fillText:♥") && operation.includes("#F472B6"),
      ),
    ).toBe(true);
    expectHealthyDraw(earlyCtx);
    expectHealthyDraw(lateCtx);
  });

  it("draws the fishing long action with a late catch payoff", () => {
    const earlyCtx = drawActorWorld({
      intentKind: "fish_at_pond",
      heldMs: 2_000,
      xPercent: 36,
      yPercent: 82,
    });
    const lateCtx = drawActorWorld({
      intentKind: "fish_at_pond",
      heldMs: 11_500,
      xPercent: 36,
      yPercent: 82,
    });

    const fishCount = (ctx: RecordedCanvasContext) =>
      ctx.drawOps.filter((operation) => operation.startsWith("fill:#FB923C"))
        .length;

    expect(
      earlyCtx.drawOps.some((operation) =>
        operation.startsWith("stroke:#6B4F3A"),
      ),
    ).toBe(true);
    expect(
      earlyCtx.drawOps.some((operation) =>
        operation.startsWith("fill:#EF4444"),
      ),
    ).toBe(true);
    expect(fishCount(lateCtx)).toBeGreaterThan(fishCount(earlyCtx));
    expectHealthyDraw(earlyCtx);
    expectHealthyDraw(lateCtx);
  });

  it("builds the snow buddy in stages during play_in_snow", () => {
    const earlyCtx = drawActorWorld({
      intentKind: "play_in_snow",
      heldMs: 1_200,
    });
    const lateCtx = drawActorWorld({
      intentKind: "play_in_snow",
      heldMs: 9_000,
    });

    const eyeCount = (ctx: RecordedCanvasContext) =>
      ctx.drawOps.filter(
        (operation) =>
          operation.startsWith("fillRect:") && operation.includes("#1E293B"),
      ).length;
    const armCount = (ctx: RecordedCanvasContext) =>
      ctx.drawOps.filter((operation) => operation.startsWith("stroke:#6B4F3A"))
        .length;

    expect(eyeCount(lateCtx)).toBeGreaterThan(eyeCount(earlyCtx));
    expect(armCount(lateCtx)).toBeGreaterThanOrEqual(armCount(earlyCtx) + 2);
    expectHealthyDraw(earlyCtx);
    expectHealthyDraw(lateCtx);
  });

  it("renders staged accents deterministically for the same held duration", () => {
    const firstCtx = drawActorWorld({
      intentKind: "build_cairn",
      heldMs: 6_500,
    });
    const secondCtx = drawActorWorld({
      intentKind: "build_cairn",
      heldMs: 6_500,
    });

    expect(secondCtx.drawOps).toEqual(firstCtx.drawOps);
  });

  it("grows the acorn pile during gather_acorns and lifts one as payoff", () => {
    const earlyCtx = drawActorWorld({
      intentKind: "gather_acorns",
      heldMs: 1_000,
    });
    const lateCtx = drawActorWorld({
      intentKind: "gather_acorns",
      heldMs: 12_000,
    });

    const acornCount = (ctx: RecordedCanvasContext) =>
      ctx.drawOps.filter((operation) => operation.startsWith("fill:#B45309"))
        .length;

    expect(acornCount(lateCtx)).toBeGreaterThan(acornCount(earlyCtx));
    expectHealthyDraw(earlyCtx);
    expectHealthyDraw(lateCtx);
  });

  it("grows the ritual sprout during seed_ritual with a sparkle payoff", () => {
    const earlyCtx = drawActorWorld({
      intentKind: "seed_ritual",
      heldMs: 1_500,
    });
    const lateCtx = drawActorWorld({
      intentKind: "seed_ritual",
      heldMs: 13_200,
    });

    const crownCount = (ctx: RecordedCanvasContext) =>
      ctx.drawOps.filter((operation) => operation.startsWith("fill:#34D399"))
        .length;
    const sparkleCount = (ctx: RecordedCanvasContext) =>
      ctx.drawOps.filter((operation) => operation.startsWith("fill:#A7F3D0"))
        .length;

    expect(crownCount(earlyCtx)).toBe(0);
    expect(crownCount(lateCtx)).toBeGreaterThan(0);
    expect(sparkleCount(lateCtx)).toBeGreaterThan(sparkleCount(earlyCtx));
    expectHealthyDraw(earlyCtx);
    expectHealthyDraw(lateCtx);
  });

  it("keeps the ocarina and umbrella accents finite and deterministic", () => {
    const ocarinaCtx = drawActorWorld({
      intentKind: "play_ocarina",
      heldMs: 8_000,
    });
    const ocarinaCtxRepeat = drawActorWorld({
      intentKind: "play_ocarina",
      heldMs: 8_000,
    });
    const umbrellaCtx = drawActorWorld({
      intentKind: "leaf_umbrella_rain",
      heldMs: 4_000,
    });
    const topCtx = drawActorWorld({
      intentKind: "spin_top",
      heldMs: 6_000,
    });

    expect(ocarinaCtxRepeat.drawOps).toEqual(ocarinaCtx.drawOps);
    expect(
      ocarinaCtx.drawOps.some((operation) =>
        operation.startsWith("fillText:♪"),
      ),
    ).toBe(true);
    expect(
      umbrellaCtx.drawOps.some((operation) =>
        operation.startsWith("fill:#4A7D40"),
      ),
    ).toBe(true);
    expect(
      topCtx.drawOps.some((operation) => operation.startsWith("fill:#C98A5B")),
    ).toBe(true);
    expectHealthyDraw(ocarinaCtx);
    expectHealthyDraw(umbrellaCtx);
    expectHealthyDraw(topCtx);
  });

  it("keeps staged care accents finite with hostile actor inputs", () => {
    const ctx = makeCanvasContext();

    drawBuddyWorld({
      ctx,
      world: makeWorld(),
      palette: PALETTES[0],
      frame: Number.POSITIVE_INFINITY,
      width: 720,
      height: 260,
      compact: false,
      reducedMotion: false,
      actor: {
        xPercent: Number.NaN,
        yPercent: Number.NEGATIVE_INFINITY,
        intentKind: "care_clean",
        travel: null,
        nowMs: Number.NaN,
        intentStartedAtMs: Number.POSITIVE_INFINITY,
      },
    });

    expectHealthyDraw(ctx);
  });

  it("lights lanterns per the world override count", () => {
    const world = makeWorld();
    const baseArgs = {
      world,
      palette: PALETTES[0],
      frame: 120,
      width: 720,
      height: 260,
      compact: false,
      reducedMotion: false,
    };

    const litHeads = (ctx: RecordedCanvasContext): number =>
      ctx.fillRectStyles.filter((style) => style === "#FDE68A").length;
    const unlitHeads = (ctx: RecordedCanvasContext): number =>
      ctx.fillRectStyles.filter((style) => style === "#475569").length;

    const dayCtx = makeCanvasContext();
    drawLanterns({ ...baseArgs, ctx: dayCtx });
    expect(litHeads(dayCtx)).toBe(0);
    expect(unlitHeads(dayCtx)).toBe(3);

    const overrideCtx = makeCanvasContext();
    drawLanterns({ ...baseArgs, ctx: overrideCtx }, 2);
    expect(litHeads(overrideCtx)).toBe(2);
    expect(unlitHeads(overrideCtx)).toBe(1);

    const dousedCtx = makeCanvasContext();
    drawLanterns({ ...baseArgs, ctx: dousedCtx }, 0);
    expect(litHeads(dousedCtx)).toBe(0);
    expect(unlitHeads(dousedCtx)).toBe(3);

    const hostileCtx = makeCanvasContext();
    drawLanterns({ ...baseArgs, ctx: hostileCtx }, Number.NaN);
    expect(litHeads(hostileCtx)).toBe(0);
    expect(unlitHeads(hostileCtx)).toBe(3);
  });
});
