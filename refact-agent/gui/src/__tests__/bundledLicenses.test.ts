import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { rollup } from "rollup";
import {
  buildBundledLicenses,
  bundledLicenses,
} from "../../vitePlugins/bundledLicenses";

const roots: string[] = [];
const mit = "MIT fixture\r\nCopyright fixture\r\nAll permission terms.\r\n";
const epl = "EPL fixture\nFull agreement with Unicode “terms”.\n";
const notices = [
  "/* Copyright (c) 2017 Kiel University\nSPDX-License-Identifier: EPL-2.0 */",
  "/* Copyright (c) 2021 Kiel University\nSPDX-License-Identifier: EPL-2.0 */",
  "/* Copyright 2020 Google LLC\nApache License, Version 2.0\nhttp://www.apache.org/licenses/LICENSE-2.0 */",
];

function fixture() {
  const root = mkdtempSync(path.join(os.tmpdir(), "bundled-licenses-"));
  roots.push(root);
  const layout = path.join(root, "node_modules/@mermaid-js/layout-elk");
  const elk = path.join(root, "node_modules/elkjs");
  mkdirSync(layout, { recursive: true });
  mkdirSync(path.join(elk, "lib"), { recursive: true });
  writeFileSync(path.join(root, "package.json"), "{}");
  for (const [directory, name, version, license, url] of [
    [
      layout,
      "@mermaid-js/layout-elk",
      "0.2.2",
      "MIT",
      "https://github.com/mermaid-js/mermaid",
    ],
    [elk, "elkjs", "0.9.3", "EPL-2.0", "https://github.com/kieler/elkjs.git"],
  ]) {
    writeFileSync(
      path.join(directory, "package.json"),
      JSON.stringify({ name, version, license, repository: { url } }),
    );
  }
  writeFileSync(path.join(layout, "LICENSE"), mit);
  writeFileSync(path.join(elk, "LICENSE.md"), epl);
  writeFileSync(
    path.join(elk, "lib/elk.bundled.js"),
    notices.join("\nthrow new Error('never execute');\n"),
  );
  return { root, layout, elk };
}

afterEach(() => {
  for (const root of roots.splice(0))
    rmSync(root, { recursive: true, force: true });
});

describe("bundled ELK license asset", () => {
  it("preserves full license bytes and every original notice deterministically", () => {
    const { layout, elk } = fixture();
    const result = buildBundledLicenses(layout, elk);
    expect(result).toBe(buildBundledLicenses(layout, elk));
    expect(result).toContain(mit);
    expect(result).toContain(epl);
    for (const notice of notices) expect(result).toContain(notice);
    expect(result).not.toContain("never execute");
    expect(result).toContain("https://github.com/kieler/elkjs/tree/0.9.3");
    expect(result).toContain(
      "Source code for elkjs is available under EPL-2.0",
    );
  });

  it.each(["LICENSE", "LICENSE.md"])("fails closed on missing %s", (file) => {
    const { layout, elk } = fixture();
    rmSync(path.join(file === "LICENSE" ? layout : elk, file));
    expect(() => buildBundledLicenses(layout, elk)).toThrow(/ENOENT/);
  });

  it("rejects absent notices and oversized bundled input", () => {
    const { layout, elk } = fixture();
    const file = path.join(elk, "lib/elk.bundled.js");
    writeFileSync(file, "throw new Error('never execute');");
    expect(() => buildBundledLicenses(layout, elk)).toThrow(/Missing expected/);
    writeFileSync(file, Buffer.alloc(16 * 1024 * 1024 + 1));
    expect(() => buildBundledLicenses(layout, elk)).toThrow(/exceeds 16 MiB/);
  });

  it("emits the expected readable asset exactly once", async () => {
    const { root } = fixture();
    const bundle = await rollup({
      input: "fixture-entry",
      plugins: [
        {
          name: "fixture-entry",
          resolveId: () => "fixture-entry",
          load: () => "export const fixture = true;",
        },
        bundledLicenses(root),
      ],
    });
    try {
      const { output } = await bundle.generate({ format: "es" });
      const assets = output.filter((entry) => entry.type === "asset");
      expect(assets).toHaveLength(1);
      expect(assets[0].fileName).toBe("BUNDLED_LICENSES.txt");
      expect(assets[0].source).toContain(
        "https://github.com/kieler/elkjs/tree/0.9.3",
      );
    } finally {
      await bundle.close();
    }
  });
});
