import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";

import { afterEach, describe, expect, it } from "vitest";
import { build, createServer } from "vite";

import refactDesign, { transformJsxSource } from "../src/index";

const tempDirectories: string[] = [];

afterEach(async () => {
  await Promise.all(
    tempDirectories.splice(0).map((directory) =>
      rm(directory, { recursive: true, force: true }),
    ),
  );
});

describe("refactDesign", () => {
  it("rejects an empty parent-origin allowlist", () => {
    expect(() => refactDesign({ allowedParentOrigins: [] })).toThrow(
      "requires at least one allowed parent origin",
    );
  });

  it("tags intrinsic JSX elements with source and component attributes", () => {
    const result = transformJsxSource({
      code: [
        "export function Greeting() {",
        '  return <section><span>Hello</span><Widget /></section>;',
        "}",
      ].join("\n"),
      id: "/workspace/src/App.tsx",
      root: "/workspace",
    });

    expect(result?.code).toContain(
      '<section data-refact-src="src/App.tsx:2:10" data-refact-cmp="Greeting">',
    );
    expect(result?.code).toContain(
      '<span data-refact-src="src/App.tsx:2:19" data-refact-cmp="Greeting">',
    );
    expect(result?.code).toContain("<Widget />");
  });

  it("does not duplicate explicit Refact attributes", () => {
    const result = transformJsxSource({
      code: 'export const App = () => <main data-refact-src="manual" />;',
      id: "/workspace/src/App.tsx",
      root: "/workspace",
    });

    expect(result?.code.match(/data-refact-src/g)).toHaveLength(1);
    expect(result?.code).toContain('data-refact-cmp="App"');
  });

  it("has no production build output or runtime injection", async () => {
    const directory = await mkdtemp(path.join(os.tmpdir(), "refact-design-build-"));
    tempDirectories.push(directory);
    await writeFile(
      path.join(directory, "index.html"),
      '<div id="root"></div><script type="module" src="/src.tsx"></script>',
    );
    await writeFile(
      path.join(directory, "src.tsx"),
      'const node = <main />; document.querySelector("#root")?.replaceChildren(String(node.type));',
    );

    await build({
      root: directory,
      logLevel: "silent",
      plugins: [
        refactDesign({ allowedParentOrigins: ["http://127.0.0.1:8001"] }),
      ],
    });

    const html = await readFile(path.join(directory, "dist/index.html"), "utf8");
    const assetName = /src="(?:\.\/|\/)?(assets\/[^"]+\.js)"/.exec(html)?.[1];
    expect(assetName).toBeDefined();
    const javascript = await readFile(path.join(directory, "dist", assetName ?? ""), "utf8");
    expect(html).not.toContain("refact-design-runtime");
    expect(javascript).not.toContain("data-refact-src");
    expect(javascript).not.toContain("refact:design-ready");
  });

  it("injects the runtime and transforms JSX only for the dev server", async () => {
    const directory = await mkdtemp(path.join(os.tmpdir(), "refact-design-dev-"));
    tempDirectories.push(directory);
    await writeFile(path.join(directory, "index.html"), '<script type="module" src="/src.tsx"></script>');
    await writeFile(path.join(directory, "src.tsx"), "export const App = () => <main />;");
    const server = await createServer({
      root: directory,
      logLevel: "silent",
      plugins: [
        refactDesign({ allowedParentOrigins: ["http://127.0.0.1:8001"] }),
      ],
      server: { middlewareMode: true },
    });

    try {
      const transformed = await server.transformRequest("/src.tsx");
      const html = await server.transformIndexHtml("/", "<main></main>");
      expect(transformed?.code).toContain('"data-refact-cmp": "App"');
      expect(html).toContain("refact-design-runtime");
    } finally {
      await server.close();
    }
  });
});
