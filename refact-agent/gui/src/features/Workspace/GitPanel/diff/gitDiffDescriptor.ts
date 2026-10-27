/**
 * Self-contained description of one diff editor tab. It is encoded into the
 * `gitdiff:` surface key so a tab can always re-render (after reload, from a
 * persisted layout, or when focused again) without any other state.
 */
export type GitDiffSide = {
  rev: string;
  path: string;
};

export type GitDiffKind = "working" | "commit";

export type GitDiffCommit = {
  oid: string;
  short_oid: string;
};

export type GitDiffDescriptor = {
  root: string;
  path: string;
  old_path: string | null;
  /** `null` when the file does not exist on this side (added / untracked). */
  left: GitDiffSide | null;
  /** `null` when the file does not exist on this side (deleted). */
  right: GitDiffSide | null;
  kind: GitDiffKind;
  commit?: GitDiffCommit;
};

export const MUTABLE_GIT_REVS: ReadonlySet<string> = new Set([
  "WORKTREE",
  "INDEX",
]);

export const isMutableGitRev = (rev: string): boolean =>
  MUTABLE_GIT_REVS.has(rev);

function toBase64Url(text: string): string {
  const bytes = new TextEncoder().encode(text);
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary)
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/, "");
}

function fromBase64Url(encoded: string): string {
  const base64 = encoded.replace(/-/g, "+").replace(/_/g, "/");
  const padded = base64 + "=".repeat((4 - (base64.length % 4)) % 4);
  const binary = atob(padded);
  const bytes = Uint8Array.from(binary, (char) => char.charCodeAt(0));
  return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
}

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

const nonEmptyString = (value: unknown): value is string =>
  typeof value === "string" && value.length > 0;

function canonicalSide(side: GitDiffSide | null): GitDiffSide | null {
  return side ? { rev: side.rev, path: side.path } : null;
}

/** Fixed key order keeps equal descriptors on one surface key. */
function canonicalDescriptor(descriptor: GitDiffDescriptor): GitDiffDescriptor {
  return {
    root: descriptor.root,
    path: descriptor.path,
    old_path: descriptor.old_path,
    left: canonicalSide(descriptor.left),
    right: canonicalSide(descriptor.right),
    kind: descriptor.kind,
    ...(descriptor.commit
      ? {
          commit: {
            oid: descriptor.commit.oid,
            short_oid: descriptor.commit.short_oid,
          },
        }
      : {}),
  };
}

export function encodeGitDiffDescriptor(descriptor: GitDiffDescriptor): string {
  return toBase64Url(JSON.stringify(canonicalDescriptor(descriptor)));
}

function parseSide(value: unknown): GitDiffSide | null | undefined {
  if (value === null) return null;
  if (!isRecord(value)) return undefined;
  if (!nonEmptyString(value.rev) || !nonEmptyString(value.path)) {
    return undefined;
  }
  return { rev: value.rev, path: value.path };
}

export function decodeGitDiffDescriptor(
  encoded: string,
): GitDiffDescriptor | null {
  if (!encoded || !/^[A-Za-z0-9_-]+$/.test(encoded)) return null;
  let raw: unknown;
  try {
    raw = JSON.parse(fromBase64Url(encoded));
  } catch {
    return null;
  }
  if (!isRecord(raw)) return null;
  const left = parseSide(raw.left);
  const right = parseSide(raw.right);
  if (
    !nonEmptyString(raw.root) ||
    !nonEmptyString(raw.path) ||
    !(raw.old_path === null || nonEmptyString(raw.old_path)) ||
    left === undefined ||
    right === undefined ||
    (left === null && right === null) ||
    (raw.kind !== "working" && raw.kind !== "commit")
  ) {
    return null;
  }
  let commit: GitDiffCommit | undefined;
  if (raw.commit !== undefined) {
    if (
      !isRecord(raw.commit) ||
      !nonEmptyString(raw.commit.oid) ||
      !nonEmptyString(raw.commit.short_oid)
    ) {
      return null;
    }
    commit = { oid: raw.commit.oid, short_oid: raw.commit.short_oid };
  }
  return {
    root: raw.root,
    path: raw.path,
    old_path: raw.old_path,
    left,
    right,
    kind: raw.kind,
    ...(commit ? { commit } : {}),
  };
}

export function gitRevLabel(
  rev: string,
  descriptor: Pick<GitDiffDescriptor, "commit">,
): string {
  if (rev === "HEAD") return "HEAD";
  if (rev === "INDEX") return "Staged";
  if (rev === "WORKTREE") return "Working tree";
  if (descriptor.commit?.oid === rev) return descriptor.commit.short_oid;
  return rev.slice(0, 7);
}

export const pathBasename = (path: string): string => {
  const normalized = path.replace(/\\/g, "/");
  return normalized.slice(normalized.lastIndexOf("/") + 1) || path;
};

export function gitDiffTitle(descriptor: GitDiffDescriptor): string {
  return pathBasename(descriptor.path);
}

export function gitDiffTooltip(descriptor: GitDiffDescriptor): string {
  const left = descriptor.left
    ? gitRevLabel(descriptor.left.rev, descriptor)
    : "(none)";
  const right = descriptor.right
    ? gitRevLabel(descriptor.right.rev, descriptor)
    : "(none)";
  const path =
    descriptor.old_path && descriptor.old_path !== descriptor.path
      ? `${descriptor.old_path} → ${descriptor.path}`
      : descriptor.path;
  return `${path} (${left} ↔ ${right})`;
}

export function joinRepoPath(root: string, path: string): string {
  if (!root) return path;
  const separator = root.includes("\\") && !root.includes("/") ? "\\" : "/";
  return root.endsWith(separator) || root.endsWith("/")
    ? `${root}${path}`
    : `${root}${separator}${path}`;
}

export function formatBytes(size: number): string {
  if (size < 1024) return `${size} B`;
  if (size < 1024 * 1024) return `${(size / 1024).toFixed(1)} KiB`;
  return `${(size / (1024 * 1024)).toFixed(1)} MiB`;
}
