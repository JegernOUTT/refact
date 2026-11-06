import { expect, test } from "vitest";
import {
  isSplittableSurface,
  isWorkspaceSurface,
  isWorkspaceSurfaceEnabled,
  makeGitDiffSurfaceKey,
} from "./surfaceKey";
const disabled = { filesPanel: false, gitPanel: false, terminalPanel: false };
test.each([
  "dashboard",
  "buddy:other",
  "buddy:",
  "task:",
  "chat:",
  "design:",
  "file:",
  "git:other",
  "gitdiff:bad",
  "files:main",
  "terminal:main",
])("rejects non-pane or malformed key %s", (key) => {
  expect(isSplittableSurface(key)).toBe(false);
  expect(isWorkspaceSurface(key)).toBe(false);
  expect(isWorkspaceSurfaceEnabled(key, disabled)).toBe(false);
});
test("gates files and Git independently, never tasks, Buddy or Design", () => {
  const diff = makeGitDiffSurfaceKey({
    root: "/repo",
    path: "a",
    old_path: null,
    left: null,
    right: { rev: "WORKTREE", path: "a" },
    kind: "working",
  });
  for (const key of ["chat:a", "task:a", "buddy:home", "design:a"])
    expect(isWorkspaceSurfaceEnabled(key, disabled)).toBe(true);
  for (const key of ["file:/a", diff, "git:main"])
    expect(isWorkspaceSurfaceEnabled(key, disabled)).toBe(false);
  expect(isWorkspaceSurfaceEnabled(diff, { ...disabled, gitPanel: true })).toBe(
    true,
  );
  expect(
    isWorkspaceSurfaceEnabled("file:/a", { ...disabled, filesPanel: true }),
  ).toBe(true);
});
