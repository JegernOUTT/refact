import { describe, expect, it, vi, type Mock } from "vitest";
import {
  drawWorldObject,
  drawWorldObjects,
} from "../features/Buddy/buddyWorldDrawObjects";
import { buildBuddyWorldState } from "../features/Buddy/buddyWorldModel";
import type { DrawBuddyWorldBaseArgs } from "../features/Buddy/buddyWorldDrawHelpers";
import { PALETTES } from "../features/Buddy/constants";
import type { BuddyPulse } from "../features/Buddy/types";

function makePulse(): BuddyPulse {
  return {
    generated_at: "2024-01-01T00:00:00Z",
    tasks: { total: 3, stuck: 1, abandoned: 0, by_status: {} },
    trajectories: { total: 10, untitled: 0, oldest_age_days: 1 },
    memory: { total: 5, orphan: 1, stale_conflicts: 0 },
    providers: { defaults_ok: true, broken_refs: 0, quota_warnings: 1 },
    mcp: { total: 4, failing: 1, auth_expiring: 0 },
    customization: { modes: 3, skills: 2, commands: 1, subagents: 0, hooks: 0 },
    diagnostics: { last_hour: 0, top_error_types: [] },
    git: { uncommitted_files: 2, diff_lines_4h: 5, branches: 3 },
    worktrees: {
      total_registered: 0,
      total_discovered: 0,
      total: 0,
      clean: 0,
      dirty: 0,
      unknown: 0,
      stale: 0,
      conflicted: 0,
      shared: 0,
      abandoned_clean: 0,
      changed_files: 0,
      additions: 0,
      deletions: 0,
      missing_registry_paths: 0,
      unregistered_cache_dirs: 0,
      merged_branches: 0,
    },
    humor: null,
  };
}

const CTX_METHODS = [
  "arc",
  "beginPath",
  "bezierCurveTo",
  "ellipse",
  "fill",
  "fillRect",
  "fillText",
  "lineTo",
  "moveTo",
  "restore",
  "save",
  "stroke",
] as const;

function makeRecordingContext(): {
  ctx: CanvasRenderingContext2D;
  mocks: Record<(typeof CTX_METHODS)[number], Mock>;
} {
  const mocks = Object.fromEntries(
    CTX_METHODS.map((method) => [method, vi.fn()]),
  ) as Record<(typeof CTX_METHODS)[number], Mock>;
  const ctx = {
    ...mocks,
    globalAlpha: 1,
    fillStyle: "#000000",
    strokeStyle: "#000000",
    lineWidth: 1,
    lineCap: "butt" as CanvasLineCap,
    font: "",
    textAlign: "center" as CanvasTextAlign,
    textBaseline: "top" as CanvasTextBaseline,
  } as unknown as CanvasRenderingContext2D;
  return { ctx, mocks };
}

function makeDrawArgs(
  ctx: CanvasRenderingContext2D,
  reducedMotion: boolean,
): DrawBuddyWorldBaseArgs {
  return {
    ctx,
    world: buildBuddyWorldState({
      now: new Date("2024-06-01T12:00:00"),
      pulse: makePulse(),
      pet: undefined,
      nowPlaying: null,
      activeQuest: null,
    }),
    palette: PALETTES[0],
    frame: 120,
    width: 800,
    height: 240,
    compact: false,
    reducedMotion,
  };
}

function expectFiniteNumericArgs(mocks: Record<string, Mock>): void {
  for (const [method, mock] of Object.entries(mocks)) {
    for (const call of mock.mock.calls) {
      for (const arg of call) {
        if (typeof arg === "number") {
          expect(
            Number.isFinite(arg),
            `${method} received non-finite number`,
          ).toBe(true);
        }
      }
    }
  }
}

describe("buddy world object drawing", () => {
  it.each([false, true])(
    "draws every world object sprite without throwing (reducedMotion=%s)",
    (reducedMotion) => {
      const { ctx, mocks } = makeRecordingContext();
      const args = makeDrawArgs(ctx, reducedMotion);

      expect(() => drawWorldObjects(args)).not.toThrow();
      expect(mocks.fillRect).toHaveBeenCalled();
      expectFiniteNumericArgs(mocks);
    },
  );

  it.each(["stats", "settings"])(
    "renders the %s landmark sprite with painted pixels",
    (objectId) => {
      const { ctx, mocks } = makeRecordingContext();
      const args = makeDrawArgs(ctx, false);
      const object = args.world.objects.find((item) => item.id === objectId);

      expect(object).toBeDefined();
      if (!object) return;

      drawWorldObject(args, object);

      expect(mocks.fillRect).toHaveBeenCalled();
      expect(mocks.ellipse).toHaveBeenCalled();
      expectFiniteNumericArgs(mocks);
    },
  );
});
