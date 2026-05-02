import { isBuddyRuntimeEventVisible } from "./buddyRuntimeEvents";
import type {
  BuddyPage,
  BuddyPetState,
  BuddyPulse,
  BuddyQuest,
  BuddyRuntimeEvent,
} from "./types";

export type BuddyWorldPhase = "morning" | "day" | "evening" | "night";
export type BuddyWorldWeather =
  | "clear"
  | "aurora"
  | "busy"
  | "wind"
  | "rain"
  | "storm"
  | "dream";

export type BuddyWorldTone = "good" | "neutral" | "warning" | "danger";
export type BuddyWorldSprite =
  | "task_grove"
  | "memory_fireflies"
  | "observatory"
  | "satellite"
  | "git_vane"
  | "market_comet"
  | "seed";

export interface BuddyWorldObject {
  id: string;
  sprite: BuddyWorldSprite;
  label: string;
  value: string;
  description: string;
  page: BuddyPage;
  tone: BuddyWorldTone;
  x: number;
  y: number;
  size: number;
}

export interface BuddyWorldState {
  phase: BuddyWorldPhase;
  phaseLabel: string;
  phaseMessage: string;
  celestialEmoji: string;
  celestialLabel: string;
  celestialAction: string;
  celestialX: number;
  celestialY: number;
  weather: BuddyWorldWeather;
  weatherLabel: string;
  weatherDescription: string;
  weatherX: number;
  weatherY: number;
  vitality: "lush" | "growing" | "tangled";
  vitalityLabel: string;
  objects: BuddyWorldObject[];
  headline: string;
}

const ACTIVE_RUNTIME_STATUSES = new Set(["started", "progress", "streaming"]);

function phaseFromHour(hour: number): BuddyWorldPhase {
  if (hour >= 5 && hour < 11) return "morning";
  if (hour >= 11 && hour < 17) return "day";
  if (hour >= 17 && hour < 21) return "evening";
  return "night";
}

function phaseDetails(
  phase: BuddyWorldPhase,
): Pick<
  BuddyWorldState,
  | "phaseLabel"
  | "phaseMessage"
  | "celestialEmoji"
  | "celestialLabel"
  | "celestialAction"
  | "celestialX"
  | "celestialY"
> {
  switch (phase) {
    case "morning":
      return {
        phaseLabel: "Morning boot glow",
        phaseMessage: "The sun is warming up the project garden.",
        celestialEmoji: "🌅",
        celestialLabel: "Sunrise",
        celestialAction: "Charge focus",
        celestialX: 18,
        celestialY: 22,
      };
    case "day":
      return {
        phaseLabel: "Daylight build mode",
        phaseMessage: "Everything is bright enough for Buddy to inspect.",
        celestialEmoji: "☀️",
        celestialLabel: "Sun",
        celestialAction: "Play in sun",
        celestialX: 48,
        celestialY: 14,
      };
    case "evening":
      return {
        phaseLabel: "Evening cooldown",
        phaseMessage: "Soft light, tidy notes, one more productive pass.",
        celestialEmoji: "🌇",
        celestialLabel: "Low sun",
        celestialAction: "Gather sparks",
        celestialX: 78,
        celestialY: 26,
      };
    case "night":
      return {
        phaseLabel: "Night daemon watch",
        phaseMessage: "The moon is up and Buddy is watching quiet queues.",
        celestialEmoji: "🌙",
        celestialLabel: "Moon",
        celestialAction: "Let Buddy rest",
        celestialX: 74,
        celestialY: 16,
      };
  }
}

function toneFromCount(
  count: number,
  warnAt: number,
  dangerAt: number,
): BuddyWorldTone {
  if (count >= dangerAt) return "danger";
  if (count >= warnAt) return "warning";
  return count > 0 ? "neutral" : "good";
}

function buildObjects(
  pulse: BuddyPulse | null | undefined,
): BuddyWorldObject[] {
  if (!pulse) {
    return [
      {
        id: "warming-up",
        sprite: "seed",
        label: "Project garden",
        value: "Warming up",
        description: "Buddy is waiting for a pulse snapshot.",
        page: { type: "buddy" },
        tone: "neutral",
        x: 25,
        y: 70,
        size: 12,
      },
    ];
  }

  const providerIssues =
    pulse.providers.broken_refs +
    pulse.providers.quota_warnings +
    (pulse.providers.defaults_ok ? 0 : 1);
  const memoryIssues = pulse.memory.orphan + pulse.memory.stale_conflicts;

  return [
    {
      id: "tasks",
      sprite: "task_grove",
      label: "Task grove",
      value: `${pulse.tasks.total} open`,
      description:
        pulse.tasks.stuck > 0
          ? `${pulse.tasks.stuck} stuck branches need Buddy's nudge.`
          : "Branches are clear enough to grow.",
      page: { type: "tasks_list" },
      tone: toneFromCount(pulse.tasks.stuck + pulse.tasks.abandoned, 1, 3),
      x: 18,
      y: 68,
      size: 16,
    },
    {
      id: "memory",
      sprite: "memory_fireflies",
      label: "Memory fireflies",
      value: `${pulse.memory.total} docs`,
      description:
        memoryIssues > 0
          ? `${memoryIssues} memory sparks want pruning.`
          : "Knowledge fireflies are neatly orbiting.",
      page: { type: "knowledge_graph" },
      tone: toneFromCount(memoryIssues, 1, 6),
      x: 33,
      y: 52,
      size: 14,
    },
    {
      id: "providers",
      sprite: "observatory",
      label: "Model observatory",
      value: pulse.providers.defaults_ok ? "Defaults ok" : "Defaults off",
      description:
        providerIssues > 0
          ? `${providerIssues} provider signals are flickering.`
          : "Model stars are aligned.",
      page: { type: "default_models" },
      tone: toneFromCount(providerIssues, 1, 3),
      x: 72,
      y: 67,
      size: 18,
    },
    {
      id: "mcp",
      sprite: "satellite",
      label: "MCP satellites",
      value: `${pulse.mcp.total} linked`,
      description:
        pulse.mcp.failing > 0 || pulse.mcp.auth_expiring > 0
          ? `${pulse.mcp.failing} failing · ${pulse.mcp.auth_expiring} auth expiring.`
          : "Satellites are holding orbit.",
      page: { type: "integrations" },
      tone: toneFromCount(pulse.mcp.failing + pulse.mcp.auth_expiring, 1, 3),
      x: 84,
      y: 35,
      size: 13,
    },
    {
      id: "git",
      sprite: "git_vane",
      label: "Git weather vane",
      value: `${pulse.git.uncommitted_files} files`,
      description:
        pulse.git.diff_lines_4h > 0
          ? `${pulse.git.diff_lines_4h} lines moved in the last 4h.`
          : "No diff winds right now.",
      page: { type: "stats" },
      tone: toneFromCount(pulse.git.uncommitted_files, 8, 20),
      x: 29,
      y: 78,
      size: 12,
    },
    {
      id: "market",
      sprite: "market_comet",
      label: "Upgrade comet",
      value: `${
        pulse.customization.skills + pulse.customization.commands
      } tools`,
      description: `${pulse.customization.modes} modes · ${pulse.customization.subagents} delegates · ${pulse.customization.hooks} hooks.`,
      page: { type: "marketplace_hub" },
      tone: "neutral",
      x: 36,
      y: 38,
      size: 13,
    },
  ];
}

function weatherFromState(
  phase: BuddyWorldPhase,
  pulse: BuddyPulse | null | undefined,
  pet: BuddyPetState | undefined,
  nowPlaying: BuddyRuntimeEvent | null,
  nowMs: number,
): Pick<
  BuddyWorldState,
  "weather" | "weatherLabel" | "weatherDescription" | "weatherX" | "weatherY"
> {
  if (pet?.condition.sleeping) {
    return {
      weather: "dream",
      weatherLabel: "Dream mist",
      weatherDescription: "Buddy is asleep; the world lowers its volume.",
      weatherX: 61,
      weatherY: 30,
    };
  }

  if (
    isBuddyRuntimeEventVisible(nowPlaying, nowMs) &&
    ACTIVE_RUNTIME_STATUSES.has(nowPlaying.status)
  ) {
    return {
      weather: "busy",
      weatherLabel: "Busy currents",
      weatherDescription: nowPlaying.title,
      weatherX: 50,
      weatherY: 61,
    };
  }

  if (pulse) {
    const stormScore =
      pulse.diagnostics.last_hour +
      pulse.providers.broken_refs * 3 +
      pulse.providers.quota_warnings * 2 +
      pulse.mcp.failing * 2 +
      (pulse.providers.defaults_ok ? 0 : 3);
    if (stormScore >= 6) {
      return {
        weather: "storm",
        weatherLabel: "Bug storm",
        weatherDescription: "Errors are crackling; Buddy can chase them down.",
        weatherX: 57,
        weatherY: 27,
      };
    }

    if (pulse.memory.orphan + pulse.memory.stale_conflicts >= 3) {
      return {
        weather: "rain",
        weatherLabel: "Memory rain",
        weatherDescription: "Old notes are watering new cleanup work.",
        weatherX: 42,
        weatherY: 28,
      };
    }

    if (pulse.git.diff_lines_4h > 0 || pulse.git.uncommitted_files > 0) {
      return {
        weather: "wind",
        weatherLabel: "Diff breeze",
        weatherDescription: "Recent changes are rustling through the garden.",
        weatherX: 44,
        weatherY: 25,
      };
    }
  }

  if (phase === "night") {
    return {
      weather: "aurora",
      weatherLabel: "Quiet aurora",
      weatherDescription: "Night signals are calm enough to sparkle.",
      weatherX: 42,
      weatherY: 24,
    };
  }

  return {
    weather: "clear",
    weatherLabel: "Clear sky",
    weatherDescription: "Buddy has room to explore and play.",
    weatherX: 42,
    weatherY: 24,
  };
}

function vitalityFromPulse(
  pulse: BuddyPulse | null | undefined,
): Pick<BuddyWorldState, "vitality" | "vitalityLabel"> {
  if (!pulse) {
    return { vitality: "growing", vitalityLabel: "Sprouting" };
  }

  const attention =
    pulse.tasks.stuck * 10 +
    pulse.tasks.abandoned * 8 +
    pulse.diagnostics.last_hour * 4 +
    pulse.providers.broken_refs * 12 +
    pulse.mcp.failing * 8 +
    pulse.memory.stale_conflicts * 6 +
    Math.min(24, pulse.git.uncommitted_files);

  if (attention >= 60) return { vitality: "tangled", vitalityLabel: "Tangled" };
  if (attention >= 20) return { vitality: "growing", vitalityLabel: "Growing" };
  return { vitality: "lush", vitalityLabel: "Lush" };
}

export function buildBuddyWorldState(args: {
  now: Date;
  pulse: BuddyPulse | null | undefined;
  pet: BuddyPetState | undefined;
  nowPlaying: BuddyRuntimeEvent | null;
  activeQuest: BuddyQuest | null;
}): BuddyWorldState {
  const phase = phaseFromHour(args.now.getHours());
  const phaseInfo = phaseDetails(phase);
  const weatherInfo = weatherFromState(
    phase,
    args.pulse,
    args.pet,
    args.nowPlaying,
    args.now.getTime(),
  );
  const vitalityInfo = vitalityFromPulse(args.pulse);
  const objects = buildObjects(args.pulse);
  const questText = args.activeQuest
    ? ` Quest active: ${args.activeQuest.title}.`
    : "";

  return {
    phase,
    ...phaseInfo,
    ...weatherInfo,
    ...vitalityInfo,
    objects,
    headline:
      `${phaseInfo.phaseMessage} ${weatherInfo.weatherDescription}${questText}`.trim(),
  };
}
