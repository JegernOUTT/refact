export const MODE_BADGE_COLORS = [
  "gray",
  "gold",
  "bronze",
  "brown",
  "yellow",
  "amber",
  "orange",
  "tomato",
  "red",
  "ruby",
  "crimson",
  "pink",
  "plum",
  "purple",
  "violet",
  "iris",
  "indigo",
  "blue",
  "cyan",
  "teal",
  "jade",
  "green",
  "grass",
  "lime",
  "mint",
  "sky",
] as const;

export type ModeBadgeColor = (typeof MODE_BADGE_COLORS)[number];

export function getModeColor(modeId: string | undefined): ModeBadgeColor {
  if (!modeId) return "blue";
  let hash = 70;
  for (let i = 0; i < modeId.length; i++) {
    hash ^= modeId.charCodeAt(i);
    hash = Math.imul(hash, 16777619);
  }
  const index = Math.abs(hash) % MODE_BADGE_COLORS.length;
  return MODE_BADGE_COLORS[index];
}
