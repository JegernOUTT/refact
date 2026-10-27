import { describe, expect, it } from "vitest";

import {
  gitDiffDescriptorFromSurfaceKey,
  isMainSurfaceEnabled,
  isWorkspaceSurface,
  makeGitDiffSurfaceKey,
  parseSurfaceKey,
} from "../../surfaceKey";
import {
  decodeGitDiffDescriptor,
  encodeGitDiffDescriptor,
  gitDiffTooltip,
  type GitDiffDescriptor,
} from "./gitDiffDescriptor";

const renamed: GitDiffDescriptor = {
  root: "/home/ünïcode repo",
  path: "src/next name.ts",
  old_path: "src/prev.ts",
  left: { rev: "HEAD", path: "src/prev.ts" },
  right: { rev: "WORKTREE", path: "src/next name.ts" },
  kind: "working",
};

const commit: GitDiffDescriptor = {
  root: "/repo",
  path: "README.md",
  old_path: null,
  left: null,
  right: { rev: "c0ffee1234", path: "README.md" },
  kind: "commit",
  commit: { oid: "c0ffee1234", short_oid: "c0ffee1" },
};

describe("git diff descriptor", () => {
  it("round-trips through base64url encoding", () => {
    for (const descriptor of [renamed, commit]) {
      const encoded = encodeGitDiffDescriptor(descriptor);
      expect(encoded).toMatch(/^[A-Za-z0-9_-]+$/);
      expect(decodeGitDiffDescriptor(encoded)).toEqual(descriptor);
    }
  });

  it("encodes equal descriptors to the same key regardless of key order", () => {
    const reordered = {
      kind: renamed.kind,
      right: renamed.right,
      left: renamed.left,
      old_path: renamed.old_path,
      path: renamed.path,
      root: renamed.root,
    } as GitDiffDescriptor;
    expect(makeGitDiffSurfaceKey(reordered)).toBe(
      makeGitDiffSurfaceKey(renamed),
    );
  });

  it("round-trips through the gitdiff surface key", () => {
    const key = makeGitDiffSurfaceKey(renamed);
    expect(key.startsWith("gitdiff:")).toBe(true);
    expect(parseSurfaceKey(key).kind).toBe("gitdiff");
    expect(gitDiffDescriptorFromSurfaceKey(key)).toEqual(renamed);
    expect(isWorkspaceSurface(key)).toBe(true);
    const caps = { filesPanel: true, gitPanel: true, terminalPanel: true };
    expect(isMainSurfaceEnabled(key, caps)).toBe(true);
    expect(isMainSurfaceEnabled(key, { ...caps, gitPanel: false })).toBe(false);
  });

  it("rejects malformed keys", () => {
    expect(decodeGitDiffDescriptor("not base64!")).toBeNull();
    expect(decodeGitDiffDescriptor(btoa('{"root":"/r"}'))).toBeNull();
    expect(() => parseSurfaceKey("gitdiff:")).toThrow("invalid surface key");
    expect(() => parseSurfaceKey("gitdiff:e30")).toThrow("invalid surface key");
    expect(gitDiffDescriptorFromSurfaceKey("file:/x")).toBeNull();
  });

  it("describes renames and revisions in the tooltip", () => {
    expect(gitDiffTooltip(renamed)).toBe(
      "src/prev.ts → src/next name.ts (HEAD ↔ Working tree)",
    );
    expect(gitDiffTooltip(commit)).toBe("README.md ((none) ↔ c0ffee1)");
  });
});
