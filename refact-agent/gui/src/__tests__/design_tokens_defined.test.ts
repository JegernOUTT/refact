import { readdir, readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

type SourceFile = { path: string; content: string };

async function collectFiles(directory: string): Promise<SourceFile[]> {
  const entries = await readdir(directory, { withFileTypes: true });
  const nested = await Promise.all(
    entries.map(async (entry) => {
      const path = resolve(directory, entry.name);
      if (entry.isDirectory()) return collectFiles(path);
      if (!entry.isFile()) return [];
      return [{ path, content: await readFile(path, "utf8") }];
    }),
  );
  return nested.flat();
}

describe("design token definitions", () => {
  it("defines every CSS token use", async () => {
    const sourceRoot = resolve(process.cwd(), "src");
    const files = await collectFiles(sourceRoot);
    const stylesheets = files.filter((file) => file.path.endsWith(".css"));
    const scripts = files.filter((file) => /\.[jt]sx?$/.test(file.path));
    const definitions = new Set<string>();

    for (const file of stylesheets) {
      for (const match of file.content.matchAll(/(--rf-[a-z0-9-]+)\s*:/g)) {
        definitions.add(match[1]);
      }
    }
    for (const file of scripts) {
      for (const match of file.content.matchAll(
        /["'](--rf-[a-z0-9-]+)["']\s*:/g,
      )) {
        definitions.add(match[1]);
      }
      for (const match of file.content.matchAll(
        /setProperty\(\s*["'](--rf-[a-z0-9-]+)["']/g,
      )) {
        definitions.add(match[1]);
      }
    }

    const undefinedUses = new Set<string>();
    for (const file of stylesheets) {
      for (const match of file.content.matchAll(
        /var\(\s*(--rf-[a-z0-9-]+)\s*([^)]*)\)/g,
      )) {
        if (match[2].includes(",") || definitions.has(match[1])) continue;
        const relativePath = file.path.slice(sourceRoot.length + 1);
        undefinedUses.add(`${match[1]}@src/${relativePath}`);
      }
    }

    const actual = [...undefinedUses].sort();
    expect(
      actual,
      `Undefined token names: ${[
        ...new Set(actual.map((pair) => pair.split("@")[0])),
      ].join(", ")}\nUndefined token pairs: ${actual.join(", ")}`,
    ).toEqual([]);
  });
});
