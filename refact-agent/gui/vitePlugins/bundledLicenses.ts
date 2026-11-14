import { closeSync, openSync, readFileSync, readSync } from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import type { Plugin } from "vite";

function requiredText(file: string): string {
  const text = readFileSync(file, "utf8");
  if (!text.trim()) throw new Error(`Empty bundled license input: ${file}`);
  return text;
}

function metadata(directory: string) {
  const file = path.join(directory, "package.json");
  const value: unknown = JSON.parse(requiredText(file));
  if (
    !value ||
    typeof value !== "object" ||
    !("name" in value) ||
    typeof value.name !== "string" ||
    !("version" in value) ||
    typeof value.version !== "string" ||
    !("license" in value) ||
    typeof value.license !== "string" ||
    !("repository" in value) ||
    !value.repository ||
    typeof value.repository !== "object" ||
    !("url" in value.repository) ||
    typeof value.repository.url !== "string"
  ) {
    throw new Error(`Invalid bundled license metadata: ${file}`);
  }
  return {
    name: value.name,
    version: value.version,
    license: value.license,
    repository: value.repository.url
      .replace(/^git\+/, "")
      .replace(/\.git$/, ""),
  };
}

function elkNotices(file: string): string {
  const limit = 16 * 1024 * 1024;
  const buffer = Buffer.alloc(limit + 1);
  const fd = openSync(file, "r");
  let length = 0;
  try {
    while (length < buffer.length) {
      const count = readSync(fd, buffer, length, buffer.length - length, null);
      if (count === 0) break;
      length += count;
    }
  } finally {
    closeSync(fd);
  }
  if (length > limit)
    throw new Error(`Bundled license input exceeds 16 MiB: ${file}`);
  const blocks = buffer
    .toString("utf8", 0, length)
    .match(/\/\*[\s\S]*?\*\//g)
    ?.filter((block) => /copyright/i.test(block));
  if (
    !blocks?.some((block) =>
      block.includes("SPDX-License-Identifier: EPL-2.0"),
    ) ||
    !blocks.some((block) => block.includes("Apache License, Version 2.0"))
  ) {
    throw new Error(`Missing expected ELK copyright/license notices: ${file}`);
  }
  return blocks.join("\n\n");
}

export function buildBundledLicenses(
  layoutDirectory: string,
  elkDirectory: string,
): string {
  const layout = metadata(layoutDirectory);
  const elk = metadata(elkDirectory);
  const mit = requiredText(path.join(layoutDirectory, "LICENSE"));
  const epl = requiredText(path.join(elkDirectory, "LICENSE.md"));
  const notices = elkNotices(path.join(elkDirectory, "lib/elk.bundled.js"));
  return (
    [
      "Bundled ELK layout dependency licenses (not a complete dependency inventory).",
      `${layout.name} ${layout.version} — ${layout.license}`,
      `Repository: ${layout.repository}`,
      `Versioned package: https://www.npmjs.com/package/${layout.name}/v/${layout.version}`,
      mit,
      `${elk.name} ${elk.version} — ${elk.license}`,
      `Source code for elkjs is available under ${elk.license}: ${elk.repository}/tree/${elk.version}`,
      "ELK Java source and build instructions are referenced by the elkjs README:",
      `${elk.repository}/blob/${elk.version}/README.md#building`,
      "https://github.com/eclipse/elk",
      "Original copyright/license notices from elkjs/lib/elk.bundled.js:",
      notices,
      epl,
    ].join("\n\n") + "\n"
  );
}

export function bundledLicenses(root: string): Plugin {
  return {
    name: "bundled-elk-licenses",
    apply: "build",
    buildStart() {
      const require = createRequire(path.join(root, "package.json"));
      const layoutPackage = require.resolve(
        "@mermaid-js/layout-elk/package.json",
      );
      const elkPackage =
        createRequire(layoutPackage).resolve("elkjs/package.json");
      this.emitFile({
        type: "asset",
        fileName: "BUNDLED_LICENSES.txt",
        source: buildBundledLicenses(
          path.dirname(layoutPackage),
          path.dirname(elkPackage),
        ),
      });
    },
  };
}
