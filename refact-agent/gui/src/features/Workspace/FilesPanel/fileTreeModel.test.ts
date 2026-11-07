import { describe, expect, it } from "vitest";
import type { FilesTreeEntry } from "../../../services/refact/files";
import { flattenVisibleTree, sortTreeEntries } from "./fileTreeModel";

const entry = (
  name: string,
  size: number | null,
  kind: "file" | "dir" = "file",
): FilesTreeEntry => ({
  name,
  path: `/root/${name}`,
  size,
  kind,
  privacy_zone: { name: "normal", send_to: ["*"], on_shell_read: "withhold" },
});

describe("Explorer ordering", () => {
  it("keeps folders first and unknown sizes last in both directions without mutating entries", () => {
    const entries = [
      entry("unknown", null),
      entry("small", 1),
      entry("folder", null, "dir"),
      entry("large", 10),
    ];
    expect(
      sortTreeEntries(entries, "size", "asc").map((item) => item.name),
    ).toEqual(["folder", "small", "large", "unknown"]);
    expect(
      sortTreeEntries(entries, "size", "desc").map((item) => item.name),
    ).toEqual(["folder", "large", "small", "unknown"]);
    expect(entries[0].name).toBe("unknown");
  });
  it("sorts each sibling group while preserving expansion and ignored filtering", () => {
    const folder = entry("folder", null, "dir");
    const expanded = new Set([folder.path]);
    expect(
      flattenVisibleTree([entry("z.ts", 1), folder], expanded, {
        [folder.path]: [
          entry("b.ts", 1),
          entry("a.ts", 2),
          { ...entry("ignored", 0), ignored: true },
        ],
      }).map((item) => [item.name, item.depth]),
    ).toEqual([
      ["folder", 0],
      ["a.ts", 1],
      ["b.ts", 1],
      ["z.ts", 0],
    ]);
    expect(expanded.has(folder.path)).toBe(true);
  });
  it("orders types by extension with deterministic path ties", () => {
    expect(
      sortTreeEntries(
        [entry("z.ts", 1), entry("a.rs", 1), entry("b.rs", 1)],
        "type",
        "desc",
      ).map((item) => item.name),
    ).toEqual(["z.ts", "a.rs", "b.rs"]);
  });
});
