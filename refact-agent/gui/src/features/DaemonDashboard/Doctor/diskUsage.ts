const BYTE_UNITS = [
  { unit: "GiB", size: 1024 ** 3 },
  { unit: "MiB", size: 1024 ** 2 },
  { unit: "KiB", size: 1024 },
] as const;

export type DiskCacheBreakdown = {
  worktrees: number;
  shadowRepos: number;
  logs: number;
  capped: boolean;
};

export function humanizeBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return `${bytes} bytes`;
  for (const { unit, size } of BYTE_UNITS) {
    if (bytes >= size) return `${(bytes / size).toFixed(1)} ${unit}`;
  }
  return `${bytes} bytes`;
}

export function humanizeByteMessage(message: string): string {
  return message.replace(/(\d+)\s+bytes/g, (raw, digits: string) => {
    const bytes = Number(digits);
    return bytes >= 1024 ? humanizeBytes(bytes) : raw;
  });
}

function parseField(detail: string, field: string): number | null {
  const match = new RegExp(`(?:^|\\s)${field}=(\\d+)(?:\\s|$)`).exec(detail);
  if (!match) return null;
  const value = Number(match[1]);
  return Number.isFinite(value) ? value : null;
}

export function parseDiskCacheDetail(
  detail: string,
): DiskCacheBreakdown | null {
  const worktrees = parseField(detail, "worktrees");
  const shadowRepos = parseField(detail, "shadow_repos");
  const logs = parseField(detail, "logs");
  if (worktrees === null || shadowRepos === null || logs === null) return null;
  return {
    worktrees,
    shadowRepos,
    logs,
    capped: /(?:^|\s)capped=true(?:\s|$)/.test(detail),
  };
}
