import { readdir, readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

type SourceFile = { path: string; content: string };

const KNOWN_UNDEFINED_TOKEN_USES = Object.freeze([
  "--rf-color-bg@src/components/Buttons/button.module.css",
  "--rf-color-text@src/features/Workspace/FilesPanel/FilesPanel.module.css",
  "--rf-disabled-opacity@src/components/AtCommands/AtCommandChip.module.css",
  "--rf-disabled-opacity@src/components/ConnectionStatus/ConnectionStatus.module.css",
  "--rf-disabled-opacity@src/features/Workspace/FilesPanel/FilesPanel.module.css",
  "--rf-dur-base@src/components/IntegrationsView/IntegrationForm/IntegrationForm.module.css",
  "--rf-dur-base@src/features/Integrations/IntegrationFormField.module.css",
  "--rf-fg@src/features/Tasks/Tasks.module.css",
  "--rf-focus-offset@src/components/AtCommands/AtCommandChip.module.css",
  "--rf-focus-offset@src/components/DialogImage/DialogImage.module.css",
  "--rf-focus-offset@src/features/Knowledge/MemoryTagFilter.module.css",
  "--rf-focus-width@src/components/AtCommands/AtCommandChip.module.css",
  "--rf-font-size-1@src/features/Tasks/CardComments/CardCommentsSection.module.css",
  "--rf-font-size-1@src/features/Tasks/Tasks.module.css",
  "--rf-font-size-2@src/features/Tasks/Tasks.module.css",
  "--rf-font-size-4@src/features/Tasks/Tasks.module.css",
  "--rf-font-size-5@src/features/Tasks/Tasks.module.css",
  "--rf-line-height-2@src/components/AtCommands/AtCommandChip.module.css",
  "--rf-line-height-normal@src/features/Tasks/Tasks.module.css",
  "--rf-line-height-tight@src/features/Tasks/Tasks.module.css",
  "--rf-line-tight@src/components/ChatForm/AttachmentTile.module.css",
  "--rf-line-tight@src/components/ChatForm/ChatForm.module.css",
  "--rf-radius-1@src/features/Workspace/FilesPanel/FilesPanel.module.css",
  "--rf-radius-2@src/features/Providers/ProviderForm/ProviderForm.module.css",
  "--rf-radius-control@src/features/Design/Design.module.css",
  "--rf-radius-sm@src/features/Customization/components/editors.module.css",
  "--rf-space-15@src/components/BackgroundAgentCard/BackgroundAgentCard.module.css",
  "--rf-surface-0@src/features/Design/Design.module.css",
  "--rf-surface-base@src/components/Reveal/reveal.module.css",
] as const);

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
  it("keeps undefined CSS token uses on a shrinking allowlist", async () => {
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
    const expected: string[] = [...KNOWN_UNDEFINED_TOKEN_USES].sort();
    const added = actual.filter((pair) => !expected.includes(pair));
    const removed = expected.filter((pair) => !actual.includes(pair));
    const addedNames = [...new Set(added.map((pair) => pair.split("@")[0]))];

    expect(
      actual,
      `Added undefined token names: ${addedNames.join(
        ", ",
      )}\nAdded undefined token pairs: ${added.join(
        ", ",
      )}\nRemoved known uses: ${removed.join(", ")}`,
    ).toEqual(expected);
  });
});
