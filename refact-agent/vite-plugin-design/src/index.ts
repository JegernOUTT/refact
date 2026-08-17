import path from "node:path";

import { parse } from "@babel/parser";
import traverseModule, { type NodePath } from "@babel/traverse";
import * as t from "@babel/types";
import MagicString from "magic-string";
import type { Plugin, ResolvedConfig } from "vite";

const VIRTUAL_RUNTIME_ID = "virtual:refact-design-runtime";
const RESOLVED_RUNTIME_ID = `\0${VIRTUAL_RUNTIME_ID}`;
const RUNTIME_BROWSER_ID = `/@id/__x00__${VIRTUAL_RUNTIME_ID}`;
const traverse =
  (traverseModule as unknown as { default?: typeof traverseModule }).default ??
  traverseModule;

export type RefactDesignPluginOptions = {
  allowedParentOrigins: readonly string[];
};

export type JsxSourceTransformOptions = {
  code: string;
  id: string;
  root: string;
};

const normalizePath = (value: string): string => value.replaceAll(path.sep, "/");

function intrinsicElementName(node: t.JSXOpeningElement): string | null {
  if (!t.isJSXIdentifier(node.name)) return null;
  const name = node.name.name;
  return name[0] === name[0]?.toLowerCase() ? name : null;
}

function namedFunction(pathValue: NodePath<t.Node>): string | null {
  if (pathValue.isFunctionDeclaration() && pathValue.node.id) {
    return pathValue.node.id.name;
  }
  if (
    (pathValue.isArrowFunctionExpression() || pathValue.isFunctionExpression()) &&
    pathValue.parentPath?.isVariableDeclarator() &&
    t.isIdentifier(pathValue.parentPath.node.id)
  ) {
    return pathValue.parentPath.node.id.name;
  }
  if (pathValue.isClassDeclaration() && pathValue.node.id) {
    return pathValue.node.id.name;
  }
  return null;
}

function componentName(pathValue: NodePath<t.JSXOpeningElement>, id: string): string {
  let current: NodePath<t.Node> | null = pathValue.parentPath;
  while (current) {
    const name = namedFunction(current);
    if (name) return name;
    current = current.parentPath;
  }
  return path.basename(id, path.extname(id));
}

function hasAttribute(node: t.JSXOpeningElement, name: string): boolean {
  return node.attributes.some(
    (attribute) =>
      t.isJSXAttribute(attribute) &&
      t.isJSXIdentifier(attribute.name) &&
      attribute.name.name === name,
  );
}

export function transformJsxSource({
  code,
  id,
  root,
}: JsxSourceTransformOptions): { code: string; map: string } | null {
  const filename = id.split("?", 1)[0];
  if (!filename || !/\.[jt]sx$/.test(filename) || filename.includes("/node_modules/")) {
    return null;
  }
  const ast = parse(code, {
    sourceType: "module",
    sourceFilename: filename,
    plugins: ["jsx", "typescript"],
  });
  const sourcePath = normalizePath(path.relative(root, filename));
  const magic = new MagicString(code);
  let changed = false;

  traverse(ast, {
    JSXOpeningElement(openingPath) {
      const elementName = intrinsicElementName(openingPath.node);
      const end = openingPath.node.name.end;
      const location = openingPath.node.loc?.start;
      if (!elementName || end === null || end === undefined || !location) return;
      const attributes: string[] = [];
      if (!hasAttribute(openingPath.node, "data-refact-src")) {
        attributes.push(
          `data-refact-src=${JSON.stringify(`${sourcePath}:${location.line}:${location.column + 1}`)}`,
        );
      }
      if (!hasAttribute(openingPath.node, "data-refact-cmp")) {
        attributes.push(
          `data-refact-cmp=${JSON.stringify(componentName(openingPath, filename))}`,
        );
      }
      if (attributes.length === 0) return;
      magic.appendLeft(end, ` ${attributes.join(" ")}`);
      changed = true;
    },
  });

  if (!changed) return null;
  return {
    code: magic.toString(),
    map: magic
      .generateMap({ hires: true, source: filename, includeContent: true })
      .toString(),
  };
}

function normalizeAllowedOrigins(origins: readonly string[]): string[] {
  const normalized = origins.map((origin) => new URL(origin).origin);
  if (normalized.length === 0) {
    throw new Error("@refact/vite-plugin-design requires at least one allowed parent origin");
  }
  return [...new Set(normalized)];
}

export default function refactDesign(
  options: RefactDesignPluginOptions,
): Plugin {
  const allowedParentOrigins = normalizeAllowedOrigins(
    options?.allowedParentOrigins ?? [],
  );
  let config: ResolvedConfig | null = null;
  let active = false;

  return {
    name: "refact-design",
    apply: "serve",
    enforce: "pre",
    configResolved(resolvedConfig) {
      config = resolvedConfig;
      active = resolvedConfig.command === "serve";
    },
    resolveId(id) {
      return active && id === VIRTUAL_RUNTIME_ID ? RESOLVED_RUNTIME_ID : null;
    },
    load(id) {
      if (!active || id !== RESOLVED_RUNTIME_ID) return null;
      return [
        'import { createDesignRuntime } from "@refact/vite-plugin-design/runtime";',
        `createDesignRuntime({ allowedParentOrigins: ${JSON.stringify(allowedParentOrigins)} });`,
      ].join("\n");
    },
    transformIndexHtml(html) {
      if (!active) return html;
      return {
        html,
        tags: [
          {
            tag: "script",
            attrs: { type: "module", src: RUNTIME_BROWSER_ID },
            injectTo: "head",
          },
        ],
      };
    },
    transform(code, id) {
      if (!active || !config) return null;
      return transformJsxSource({ code, id, root: config.root });
    },
  };
}
