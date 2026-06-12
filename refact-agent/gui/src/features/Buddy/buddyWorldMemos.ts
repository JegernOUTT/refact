export interface BuddyWorldMemos {
  lastArcDates: Record<string, string>;
  seasonFirstsSeen: string[];
  lastSeasonSeen: string | null;
  shiroIntroSeen: boolean;
}

export const BUDDY_WORLD_MEMOS_KEY = "refact.buddy.worldMemos";

export function createEmptyBuddyWorldMemos(): BuddyWorldMemos {
  return {
    lastArcDates: {},
    seasonFirstsSeen: [],
    lastSeasonSeen: null,
    shiroIntroSeen: false,
  };
}

function defaultStorage(): Storage | null {
  if (typeof window === "undefined") return null;
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

function sanitizeMemos(raw: unknown): BuddyWorldMemos {
  const memos = createEmptyBuddyWorldMemos();
  if (typeof raw !== "object" || raw === null) return memos;
  const record = raw as Record<string, unknown>;
  if (typeof record.lastArcDates === "object" && record.lastArcDates !== null) {
    for (const [key, value] of Object.entries(
      record.lastArcDates as Record<string, unknown>,
    )) {
      if (typeof value === "string") memos.lastArcDates[key] = value;
    }
  }
  if (Array.isArray(record.seasonFirstsSeen)) {
    memos.seasonFirstsSeen = record.seasonFirstsSeen.filter(
      (entry): entry is string => typeof entry === "string",
    );
  }
  if (typeof record.lastSeasonSeen === "string") {
    memos.lastSeasonSeen = record.lastSeasonSeen;
  }
  if (typeof record.shiroIntroSeen === "boolean") {
    memos.shiroIntroSeen = record.shiroIntroSeen;
  }
  return memos;
}

export function readBuddyWorldMemos(
  storage: Storage | null = defaultStorage(),
): BuddyWorldMemos {
  if (!storage) return createEmptyBuddyWorldMemos();
  try {
    const raw = storage.getItem(BUDDY_WORLD_MEMOS_KEY);
    if (raw === null) return createEmptyBuddyWorldMemos();
    return sanitizeMemos(JSON.parse(raw));
  } catch {
    return createEmptyBuddyWorldMemos();
  }
}

export function writeBuddyWorldMemos(
  patch: Partial<BuddyWorldMemos>,
  storage: Storage | null = defaultStorage(),
): BuddyWorldMemos {
  const current = readBuddyWorldMemos(storage);
  const next: BuddyWorldMemos = {
    lastArcDates: { ...current.lastArcDates, ...(patch.lastArcDates ?? {}) },
    seasonFirstsSeen: patch.seasonFirstsSeen
      ? [...new Set([...current.seasonFirstsSeen, ...patch.seasonFirstsSeen])]
      : current.seasonFirstsSeen,
    lastSeasonSeen: patch.lastSeasonSeen ?? current.lastSeasonSeen,
    shiroIntroSeen: patch.shiroIntroSeen ?? current.shiroIntroSeen,
  };
  if (storage) {
    try {
      storage.setItem(BUDDY_WORLD_MEMOS_KEY, JSON.stringify(next));
    } catch {
      return next;
    }
  }
  return next;
}

export function buddyWorldDayKey(now: Date): string {
  const year = now.getFullYear();
  const month = `${now.getMonth() + 1}`.padStart(2, "0");
  const day = `${now.getDate()}`.padStart(2, "0");
  return `${year}-${month}-${day}`;
}
