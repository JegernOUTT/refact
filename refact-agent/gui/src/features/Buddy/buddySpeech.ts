import type { BuddyShowcaseKind, BuddySpeechStyle } from "./types";
import type { BuddyWorldIntentKind } from "./buddyWorldDirector";

export type { BuddySpeechStyle };

export type BuddySpeechLineFactory = (name: string) => string;

export interface BuddySpeechPool {
  style: BuddySpeechStyle;
  lines: readonly BuddySpeechLineFactory[];
}

export interface BuddySpeechBeat {
  atMs: number;
  style: BuddySpeechStyle;
  poolKey: string;
  lines: readonly BuddySpeechLineFactory[];
}

export interface BuddySpeechMemory {
  recent: string[];
}

export const BUDDY_SPEECH_RECENT_LIMIT = 12;

export function createBuddySpeechMemory(): BuddySpeechMemory {
  return { recent: [] };
}

function rememberSpeechLine(memory: BuddySpeechMemory, lineId: string): void {
  memory.recent = [
    ...memory.recent.filter((recentId) => recentId !== lineId),
    lineId,
  ].slice(-BUDDY_SPEECH_RECENT_LIMIT);
}

export function pickBuddySpeechLine(
  memory: BuddySpeechMemory,
  poolKey: string,
  lines: readonly BuddySpeechLineFactory[],
  name: string,
  random: () => number = Math.random,
): string {
  if (lines.length === 0) return "";
  const lineIds = lines.map((_, index) => `${poolKey}:${index}`);
  const freshIds = lineIds.filter((id) => !memory.recent.includes(id));
  const roll = Math.min(0.999999, Math.max(0, random()));
  const chosenId =
    freshIds.length > 0
      ? freshIds[Math.floor(roll * freshIds.length)]
      : [...lineIds].sort(
          (left, right) =>
            memory.recent.indexOf(left) - memory.recent.indexOf(right),
        )[0];
  const lineIndex = Number(chosenId.slice(poolKey.length + 1));
  rememberSpeechLine(memory, chosenId);
  const factory = lines[lineIndex] ?? lines[0];
  return factory(name);
}

export type BuddyWorldSpeechSource =
  | "active"
  | "care"
  | "session"
  | "arc"
  | "showcase"
  | "director"
  | "reaction"
  | "none";

export interface BuddyWorldSpeechCandidate {
  text: string;
  style: BuddySpeechStyle;
}

export interface BuddyWorldSpeechResolution {
  text: string | null;
  style: BuddySpeechStyle;
  source: BuddyWorldSpeechSource;
}

export interface ResolveBuddyWorldSpeechArgs {
  backend: BuddyWorldSpeechCandidate | null;
  care: BuddyWorldSpeechCandidate | null;
  session: BuddyWorldSpeechCandidate | null;
  arc: BuddyWorldSpeechCandidate | null;
  showcase: BuddyWorldSpeechCandidate | null;
  director: BuddyWorldSpeechCandidate | null;
  reaction: BuddyWorldSpeechCandidate | null;
}

export const BUDDY_WORLD_SPEECH_PRIORITY =
  "backend-care-session-arc-showcase-director-local";

const SPEECH_LADDER = [
  ["active", "backend"],
  ["care", "care"],
  ["session", "session"],
  ["arc", "arc"],
  ["showcase", "showcase"],
  ["director", "director"],
  ["reaction", "reaction"],
] as const satisfies readonly (readonly [
  BuddyWorldSpeechSource,
  keyof ResolveBuddyWorldSpeechArgs,
])[];

export function resolveBuddyWorldSpeech(
  args: ResolveBuddyWorldSpeechArgs,
): BuddyWorldSpeechResolution {
  for (const [source, slot] of SPEECH_LADDER) {
    const candidate = args[slot];
    if (candidate && candidate.text.length > 0) {
      return { text: candidate.text, style: candidate.style, source };
    }
  }
  return { text: null, style: "say", source: "none" };
}

export function styleForBuddySpeechIntent(
  intent: string | null | undefined,
): BuddySpeechStyle {
  const normalized = intent?.trim().toLowerCase() ?? "";
  if (normalized.length === 0) return "say";
  if (/warn|alert|error|critical|urgent|risk/.test(normalized)) return "alert";
  if (/celebrat|win|success|excite|payoff/.test(normalized)) return "excite";
  if (/dream|think|reflect|muse|wonder/.test(normalized)) return "think";
  return "say";
}

export const DIRECTOR_SPEECH_POOLS: Partial<
  Record<BuddyWorldIntentKind, BuddySpeechPool>
> = {
  morning_stretch: {
    style: "say",
    lines: [
      () => "Morning stretch. Systems: squeaky but ready.",
      () => "Big stretch. The dew tickles.",
      () => "Paws up, sun's up. Gentle plans only.",
    ],
  },
  evening_tidy: {
    style: "say",
    lines: [
      () => "Evening tidy. I’m tucking stray sparks in.",
      () => "Sweeping the day into a neat little pile.",
      () => "Last chores. The fireflies supervise.",
    ],
  },
  night_watch: {
    style: "say",
    lines: [
      () => "Night watch mode. I’ll keep the constellations tidy.",
      () => "Professor the owl and I split the night shift.",
      () => "Quiet hours. The stars hum in low voltage.",
    ],
  },
  rest_home: {
    style: "say",
    lines: [
      () => "Dream mist accepted. I’ll keep one eye on the hearth.",
      () => "Recharging by the hearth. Wake me for snacks.",
      () => "Home smells like moss and warm lamplight.",
    ],
  },
  inspect_memory: {
    style: "say",
    lines: [
      () => "I’m gathering loose memory sparks.",
      () => "These sparks remember things I forgot. Rude.",
      () => "Sorting glow-thoughts by warmth.",
    ],
  },
  shelve_memory: {
    style: "say",
    lines: [
      () => "These fireflies want a shelf.",
      () => "Filing fireflies. Alphabetical by sparkle.",
      () => "Every memory gets a cozy jar tonight.",
    ],
  },
  inspect_provider: {
    style: "alert",
    lines: [
      () => "The model stars are flickering; I’m checking the observatory.",
      () => "Observatory rounds. One star is being dramatic.",
      () => "Tuning the telescope toward the noisy constellation.",
    ],
  },
  stabilize_crystal: {
    style: "alert",
    lines: [
      () => "I’m nudging the crystal back into tune.",
      () => "Steady… steady… the crystal hates Mondays.",
      () => "Re-humming the crystal’s favorite frequency.",
    ],
  },
  channel_runtime: {
    style: "say",
    lines: [
      () => "The runes are compiling something shiny.",
      () => "I’m feeding the little spellforge.",
      () => "Work hum detected. Stirring the rune pot.",
    ],
  },
  watch_observatory: {
    style: "say",
    lines: [
      () => "I’m counting the quiet model stars.",
      () => "All constellations accounted for. Probably.",
      () => "The telescope and I are having a moment.",
    ],
  },
  seek_food: {
    style: "say",
    lines: [
      () => "Snack beacon detected.",
      () => "My stomach filed an urgent ticket.",
      () => "Following the noodle aroma at a dignified sprint.",
    ],
  },
  seek_toy: {
    style: "say",
    lines: [
      () => "The toy nook is making mysterious eye contact.",
      () => "One quick game. Okay, maybe five.",
      () => "The ball rolled first. This is self-defense.",
    ],
  },
  receive_affection: {
    style: "say",
    lines: [
      () => "Pocket warmth received. I’m glowing responsibly.",
      () => "Affection levels: cozy and climbing.",
      () => "I will accept exactly one thousand pats.",
    ],
  },
  wander_curiously: {
    style: "say",
    lines: [
      () => "I’m checking the sparkle map.",
      () => "Patrolling the clearing for new smells.",
      () => "Just wandering. Professionally.",
    ],
  },
  celebrate_recovery: {
    style: "excite",
    lines: [
      () => "Tiny recovery sparkle. Everything hums steadier now.",
      () => "Crisis over. Snacks for everyone.",
      () => "The world wobbled and we un-wobbled it.",
    ],
  },
  check_mailbox: {
    style: "excite",
    lines: [
      () => "The quest mailbox flag is up. New orders inside!",
      () => "Mail! Possibly a quest. Possibly leaves. Both fine.",
      () => "The flag’s up! I love a flag.",
    ],
  },
  warm_by_fire: {
    style: "say",
    lines: [
      () => "Campfire status: crackling within parameters.",
      () => "Toasting my toes. Official business.",
      () => "The embers are telling slow orange stories.",
    ],
  },
  watch_shooting_star: {
    style: "excite",
    lines: [
      () => "A star just zipped across the sky. Wish logged.",
      () => "Another one! The sky is showing off.",
      () => "Quick, wish for snacks. I did.",
    ],
  },
  play_in_snow: {
    style: "say",
    lines: [
      () => "Snow! I’m sculpting a tiny code angel.",
      () => "Snow crunch acoustics: ten out of ten.",
      () => "Building a snow twin. It’s very quiet company.",
    ],
  },
  collect_leaves: {
    style: "say",
    lines: [
      () => "Collecting the crunchiest leaves for the archive.",
      () => "This leaf is somehow crunchier than the last.",
      () => "Leaf inventory: growing, gorgeous, slightly damp.",
    ],
  },
  smell_flowers: {
    style: "say",
    lines: [
      () => "Petal report: fragrant and non-blocking.",
      () => "This flower smells like spring’s first morning.",
      () => "Nose-first into the petals. Zero regrets.",
    ],
  },
  tend_garden: {
    style: "say",
    lines: [
      () => "Watering the task sprouts back to green.",
      () => "Little sprouts, big dreams. Drink up.",
      () => "Garden rounds. The weeds signed a treaty.",
    ],
  },
  chase_butterfly: {
    style: "excite",
    lines: [
      () => "A butterfly! Critical chase business.",
      () => "It flew in a spiral. I respect the chaos.",
      () => "The butterfly is winning. For now.",
    ],
  },
  watch_birds: {
    style: "say",
    lines: [
      () => "Bird patrol overhead. All wings accounted for.",
      () => "The flock drew a V. Show-offs.",
      () => "Counting birds. Lost count. Restarting.",
    ],
  },
  visit_pond: {
    style: "say",
    lines: [
      () => "The koi shared confidential pond gossip.",
      () => "Brick surfaced just to say hello. Soft legend.",
      () => "Pond check. Brick says the water’s fine.",
    ],
  },
  splash_puddles: {
    style: "excite",
    lines: [
      () => "Puddle physics research. Very important.",
      () => "Each puddle splashes differently. Science!",
      () => "Wet feet, full heart.",
    ],
  },
  nap_under_tree: {
    style: "think",
    lines: [
      () => "The leaf shade is perfect. Quick recharge nap.",
      () => "Dappled light naps are premium naps.",
      () => "The great tree said shhh. I obey.",
    ],
  },
  greet_kodama: {
    style: "whisper",
    lines: [
      () => "The little forest spirits are out. Waving politely.",
      () => "The kodama clicked hello. I clicked back.",
      () => "Tiny spirits, tiny bows. Big honor.",
    ],
  },
  chase_soot_sprites: {
    style: "excite",
    lines: [
      () => "Soot sprites!! Tiny, fast, suspicious.",
      () => "The soot gang scattered. I’ll pretend I won.",
      () => "One soot sprite stole my dignity. Acceptable.",
    ],
  },
  fish_at_pond: {
    style: "say",
    lines: [
      () => "Fishing protocol engaged. The koi are negotiating terms.",
      () => "Casting in. Brick is judging my form.",
      () => "Quiet rod, quiet heart. Fish secrets incoming.",
    ],
  },
  build_cairn: {
    style: "say",
    lines: [
      () => "Stacking zen stones. Nobody breathe near the tower.",
      () => "Stone three is the betrayer. Always stone three.",
      () => "Cairn engineering: patience plus pebbles.",
    ],
  },
  catch_fireflies: {
    style: "say",
    lines: [
      () => "Recruiting lantern volunteers. Gently. With a jar.",
      () => "The fireflies blink in code. I’m decoding.",
      () => "Lantern recruitment drive: glowing reviews.",
    ],
  },
  paint_meadow: {
    style: "say",
    lines: [
      () => "Plein air session. The meadow demands more green.",
      () => "Painting the meadow. The meadow keeps moving.",
      () => "Brush loaded with sunset. No mistakes, only clouds.",
    ],
  },
  picnic_snack: {
    style: "say",
    lines: [
      () => "Tiny picnic deployed. Crumb security is, frankly, lax.",
      () => "Picnic protocol: one bite for me, one for the ants.",
      () => "The blanket is 80% crumbs now. Success.",
    ],
  },
  gather_acorns: {
    style: "say",
    lines: [
      () => "Acorn harvest! Every pocket is an acorn pocket now.",
      () => "Kuro is eyeing my acorns. Not today, crow.",
      () => "Acorn count: enough. Acorn want: more.",
    ],
  },
  leaf_umbrella_rain: {
    style: "say",
    lines: [
      () => "Leaf umbrella deployed. Dry-ish and very dignified.",
      () => "The rain drums on my leaf. Decent rhythm.",
      () => "Mostly dry under here. The puddles can wait.",
    ],
  },
  play_ocarina: {
    style: "sing",
    lines: [
      () => "Moon song time. The fireflies requested an encore.",
      () => "Ocarina hour. Professor hooted in harmony.",
      () => "Soft notes for a soft night.",
    ],
  },
  seed_ritual: {
    style: "whisper",
    lines: [
      () => "Grow, grow, grow… tiny forest ritual in progress.",
      () => "Moonlight watering. The seed glows back.",
      () => "Whispering growth spells. The sprout listens.",
    ],
  },
  spin_top: {
    style: "say",
    lines: [
      () => "Spinning top tournament. Current champion: the top.",
      () => "The top wobbled but did not fall. Inspiring.",
      () => "Best of five. The top cheats.",
    ],
  },
};

export const DIRECTOR_SPEECH_BEATS: Partial<
  Record<BuddyWorldIntentKind, readonly BuddySpeechBeat[]>
> = {
  fish_at_pond: [
    {
      atMs: 7_500,
      style: "whisper",
      poolKey: "beat:fish_at_pond:mid",
      lines: [
        () => "…something nibbles.",
        () => "…ripples. Stay very still.",
        () => "…Brick is circling the bait.",
      ],
    },
    {
      atMs: 14_800,
      style: "excite",
      poolKey: "beat:fish_at_pond:payoff",
      lines: [
        () => "Got one! Brick sent a cousin.",
        () => "Catch! Release. Respect.",
        () => "A fish! It waved. I waved back.",
      ],
    },
  ],
  build_cairn: [
    {
      atMs: 7_000,
      style: "whisper",
      poolKey: "beat:build_cairn:mid",
      lines: [
        () => "Stone four… steady…",
        () => "Don’t wobble. Don’t you dare wobble.",
      ],
    },
    {
      atMs: 13_500,
      style: "excite",
      poolKey: "beat:build_cairn:payoff",
      lines: [
        () => "The tower stands! Five stones of glory.",
        () => "Cairn complete. Architecture!",
      ],
    },
  ],
  paint_meadow: [
    {
      atMs: 8_000,
      style: "whisper",
      poolKey: "beat:paint_meadow:mid",
      lines: [
        () => "Mixing cloud-white with meadow-green…",
        () => "The brush knows the way now.",
      ],
    },
    {
      atMs: 15_000,
      style: "say",
      poolKey: "beat:paint_meadow:payoff",
      lines: [
        () => "Masterpiece-ish. Signed with a paw.",
        () => "Done! The meadow approves of its portrait.",
      ],
    },
  ],
  catch_fireflies: [
    {
      atMs: 7_000,
      style: "whisper",
      poolKey: "beat:catch_fireflies:mid",
      lines: [
        () => "Easy… the jar is friendly…",
        () => "One blink closer… closer…",
      ],
    },
    {
      atMs: 13_000,
      style: "excite",
      poolKey: "beat:catch_fireflies:payoff",
      lines: [
        () => "Three lanterns aboard! Gentle release soon.",
        () => "Caught and counted. Lanterns, assemble!",
      ],
    },
  ],
  gather_acorns: [
    {
      atMs: 7_000,
      style: "say",
      poolKey: "beat:gather_acorns:mid",
      lines: [
        () => "Pocket two is full. Pocket three: loading.",
        () => "This acorn is perfectly round. Keeper.",
      ],
    },
    {
      atMs: 13_000,
      style: "excite",
      poolKey: "beat:gather_acorns:payoff",
      lines: [() => "Harvest haul secured!", () => "Acorn mountain achieved."],
    },
  ],
  play_ocarina: [
    {
      atMs: 6_000,
      style: "sing",
      poolKey: "beat:play_ocarina:mid",
      lines: [
        () => "♪ moon, meadow, slow river ♪",
        () => "♪ leaves down, stars up ♪",
      ],
    },
    {
      atMs: 12_500,
      style: "sing",
      poolKey: "beat:play_ocarina:payoff",
      lines: [
        () => "♪ one more verse for the owl ♪",
        () => "♪ fading on a soft note ♪",
      ],
    },
  ],
  seed_ritual: [
    {
      atMs: 8_000,
      style: "whisper",
      poolKey: "beat:seed_ritual:mid",
      lines: [() => "grow… grow… tiny leaf, big sky…"],
    },
    {
      atMs: 14_500,
      style: "excite",
      poolKey: "beat:seed_ritual:payoff",
      lines: [() => "It sprouted a whole millimeter! Champion."],
    },
  ],
  picnic_snack: [
    {
      atMs: 6_000,
      style: "whisper",
      poolKey: "beat:picnic_snack:mid",
      lines: [() => "crunch… crunch… crumb tax paid."],
    },
    {
      atMs: 10_500,
      style: "say",
      poolKey: "beat:picnic_snack:payoff",
      lines: [() => "Picnic complete. The ants send regards."],
    },
  ],
  spin_top: [
    {
      atMs: 6_500,
      style: "whisper",
      poolKey: "beat:spin_top:mid",
      lines: [() => "spin… spin… don’t stop now…"],
    },
    {
      atMs: 11_500,
      style: "excite",
      poolKey: "beat:spin_top:payoff",
      lines: [() => "NEW RECORD. The top is unstoppable."],
    },
  ],
};

export const SHOWCASE_SPEECH_POOLS: Record<BuddyShowcaseKind, BuddySpeechPool> =
  {
    memory_firefly_night: {
      style: "think",
      lines: [
        (name) => `${name} gathers the memory fireflies into a soft night map.`,
        (name) => `${name} hums while the memory lights settle into rows.`,
        (name) => `${name} reads tiny glowing footnotes in the dark.`,
      ],
    },
    stargazing_constellation: {
      style: "say",
      lines: [
        (name) =>
          `${name} reads the model stars and traces a careful constellation.`,
        (name) => `${name} maps the noisy star back to its quiet orbit.`,
        (name) => `${name} squints at the telescope, then nods knowingly.`,
      ],
    },
    rain_shelter_dash: {
      style: "excite",
      lines: [
        (name) =>
          `${name} dashes under the awning and watches the rain curtain.`,
        (name) => `${name} beats the rain by half a whisker. Victory shake.`,
        (name) => `${name} counts raindrops from the dry side of the porch.`,
      ],
    },
    koi_pond_watch: {
      style: "say",
      lines: [
        (name) => `${name} holds a koi summit at the pond's edge.`,
        (name) => `${name} and Brick the koi exchange formal bubbles.`,
        (name) => `${name} listens carefully to Brick's pond report.`,
      ],
    },
    campfire_story: {
      style: "say",
      lines: [
        (name) => `${name} tells the embers a tiny heroic story.`,
        (name) => `${name} narrates the day's quest to a very warm audience.`,
        (name) => `${name} saves the story's twist for the tallest flame.`,
      ],
    },
    firefly_meadow_chase: {
      style: "excite",
      lines: [
        (name) => `${name} chases meadow fireflies in happy zigzags.`,
        (name) => `${name} loses the race to a firefly and grins anyway.`,
        (name) => `${name} herds sparkles. The sparkles disagree. Joyfully.`,
      ],
    },
    snow_sculpting: {
      style: "say",
      lines: [
        (name) => `${name} sculpts a tiny snow twin. It approves.`,
        (name) => `${name} gives the snow twin better ears. Artistic license.`,
        (name) => `${name} and the snow twin share a long, quiet stare.`,
      ],
    },
    leaf_storm_play: {
      style: "excite",
      lines: [
        (name) => `${name} leaps through a spiral of autumn leaves.`,
        (name) => `${name} conducts the leaf storm like a tiny orchestra.`,
        (name) => `${name} catches the crunchiest leaf mid-air. Trophy.`,
      ],
    },
    aurora_dance: {
      style: "sing",
      lines: [
        (name) => `${name} dances with the aurora ribbons.`,
        (name) => `${name} mirrors the sky's slow green waltz.`,
        (name) => `${name} sways while the aurora hums above.`,
      ],
    },
    komorebi_nap: {
      style: "think",
      lines: [
        (name) => `${name} naps in the dappled light under the great tree.`,
        (name) => `${name} naps where the sun leaks through the leaves.`,
        (name) => `${name} dozes in a warm puddle of komorebi.`,
      ],
    },
  };

export function careMidBeatAtMs(travelMs: number, performMs: number): number {
  return Math.max(0, travelMs) + Math.max(0, performMs) * 0.45;
}
