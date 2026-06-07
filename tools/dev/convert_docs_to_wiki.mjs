#!/usr/bin/env node
import fs from "fs";
import path from "path";

const [srcDirArg, outDirArg] = process.argv.slice(2);

if (!srcDirArg || !outDirArg) {
  console.error("usage: node tools/dev/convert_docs_to_wiki.mjs <srcDir> <outDir>");
  process.exit(1);
}

const srcDir = path.resolve(srcDirArg);
const outDir = path.resolve(outDirArg);
const docsRoot = path.resolve(srcDir, "../../..");

const explicitMap = new Map([
  ["index.mdx", "Home.md"],
  ["introduction/quickstart.md", "Quickstart.md"],
  ["installation/installation-hub.mdx", "Installation.md"],
  ["installation/vs-code.md", "Installation-VS-Code.md"],
  ["installation/jetbrains.md", "Installation-JetBrains.md"],
  ["byok.md", "BYOK.md"],
  ["supported-models.md", "Supported-Models.md"],
  ["privacy.md", "Privacy.md"],
  ["faq.md", "FAQ.md"],
  ["contributing.md", "Contributing.md"],
  ["roles.md", "Hidden-Roles.md"],
  ["scheduler.md", "Scheduler.md"],
  ["processes.md", "Processes-and-PTY.md"],
  ["features/ai-chat.md", "AI-Chat.md"],
  ["features/code-completion.md", "Code-Completion.md"],
  ["features/context.md", "Context.md"],
  ["features/ai-toolbox.md", "AI-Toolbox.md"],
  ["features/agent-integrations.md", "Agent-Integrations.md"],
  ["features/ai-toolbox/comments.md", "AI-Toolbox-Comments.md"],
  ["features/ai-toolbox/debug.md", "AI-Toolbox-Debug.md"],
  ["features/ai-toolbox/explain-code.md", "AI-Toolbox-Explain-Code.md"],
  ["features/ai-toolbox/fix-bugs.md", "AI-Toolbox-Fix-Bugs.md"],
  ["features/ai-toolbox/improve-code.md", "AI-Toolbox-Improve-Code.md"],
  ["features/ai-toolbox/naming.md", "AI-Toolbox-Naming.md"],
  ["features/autonomous-agent/getting-started.md", "Agent-Getting-Started.md"],
  ["features/autonomous-agent/overview.md", "Agent-Overview.md"],
  ["features/autonomous-agent/tools.md", "Agent-Tools.md"],
  ["features/autonomous-agent/rollback.md", "Agent-Rollback.md"],
  ["features/autonomous-agent/worktrees.md", "Agent-Worktrees.md"],
  ["features/autonomous-agent/integrations/index.md", "Integrations.md"],
  ["features/autonomous-agent/integrations/chrome.md", "Integrations-Chrome.md"],
  ["features/autonomous-agent/integrations/shell-commands.md", "Integrations-Shell-Commands.md"],
  ["features/autonomous-agent/integrations/command-line-tool.md", "Integrations-Command-Line-Tool.md"],
  ["features/autonomous-agent/integrations/command-line-service.md", "Integrations-Command-Line-Service.md"],
  ["features/autonomous-agent/integrations/mcp.md", "Integrations-MCP.md"],
  ["features/autonomous-agent/integrations/github.md", "Integrations-GitHub.md"],
  ["features/autonomous-agent/integrations/gitlab.md", "Integrations-GitLab.md"],
  ["features/autonomous-agent/integrations/bitbucket.md", "Integrations-Bitbucket.md"],
  ["features/autonomous-agent/integrations/postgresql.md", "Integrations-PostgreSQL.md"],
  ["features/autonomous-agent/integrations/mysql.md", "Integrations-MySQL.md"],
  ["features/autonomous-agent/integrations/pdb.md", "Integrations-PDB.md"],
  ["guides/plugins/jetbrains/troubleshooting.md", "JetBrains-Troubleshooting.md"],
  ["docs/BROWSER_MODE.md", "Browser-Automation.md"],
]);

const routeToFile = new Map();
const docPathToFile = new Map(explicitMap);

for (const [docPath, fileName] of explicitMap) {
  if (docPath.startsWith("docs/")) {
    continue;
  }
  const route = docPath === "index.mdx" ? "/" : `/${docPath.replace(/\.(md|mdx)$/u, "")}/`;
  routeToFile.set(route, fileName);
  routeToFile.set(route.replace(/\/$/u, ""), fileName);
  if (docPath.endsWith("/index.md") || docPath.endsWith("/index.mdx")) {
    const indexRoute = `/${path.posix.dirname(docPath)}/`;
    routeToFile.set(indexRoute, fileName);
    routeToFile.set(indexRoute.replace(/\/$/u, ""), fileName);
  }
}

function toPosix(filePath) {
  return filePath.split(path.sep).join("/");
}

function titleCasePart(value) {
  return value
    .split(/[-_\s]+/u)
    .filter(Boolean)
    .map((part) => {
      const lower = part.toLowerCase();
      if (["ai", "byok", "mcp", "pdb", "pty", "faq", "mysql"].includes(lower)) {
        return lower.toUpperCase();
      }
      if (lower === "postgresql") {
        return "PostgreSQL";
      }
      if (lower === "github") {
        return "GitHub";
      }
      if (lower === "gitlab") {
        return "GitLab";
      }
      return lower.charAt(0).toUpperCase() + lower.slice(1);
    })
    .join("-");
}

function deriveWikiFileName(relPath) {
  const parsed = path.posix.parse(relPath);
  const parts = parsed.dir.split("/").filter(Boolean);
  const leaf = parsed.name === "index" && parts.length ? parts.pop() : parsed.name;
  const prefix = parts
    .filter((part) => !["features", "guides", "plugins"].includes(part))
    .map(titleCasePart);
  const pageParts = [...prefix, titleCasePart(leaf)].filter(Boolean);
  return `${pageParts.join("-") || "Page"}.md`;
}

function walk(dir) {
  const entries = fs.readdirSync(dir, { withFileTypes: true });
  const files = [];
  for (const entry of entries) {
    const fullPath = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      files.push(...walk(fullPath));
    } else if (/\.(md|mdx)$/u.test(entry.name)) {
      files.push(fullPath);
    }
  }
  return files.sort();
}

function parseFrontmatter(text, relPath) {
  if (!text.startsWith("---\n")) {
    return { title: titleCasePart(path.posix.parse(relPath).name).replace(/-/gu, " "), description: "", body: text };
  }
  const end = text.indexOf("\n---", 4);
  if (end === -1) {
    console.error(`warning: ${relPath}: unterminated frontmatter; passing through`);
    return { title: titleCasePart(path.posix.parse(relPath).name).replace(/-/gu, " "), description: "", body: text };
  }
  const raw = text.slice(4, end).trim();
  const body = text.slice(end + 5).replace(/^\n/u, "");
  const data = {};
  for (const line of raw.split("\n")) {
    const match = line.match(/^([A-Za-z0-9_-]+):\s*(.*)$/u);
    if (match) {
      data[match[1]] = match[2].replace(/^['"]|['"]$/gu, "").trim();
    }
  }
  return { title: data.title || titleCasePart(path.posix.parse(relPath).name).replace(/-/gu, " "), description: data.description || "", body };
}

function convertMdx(text, relPath) {
  let output = text.replace(/^\s*import\s+\{[^\n]+\}\s+from\s+['"]@astrojs\/starlight\/components['"];?\s*$/gmu, "");
  output = output.replace(/<CardGrid>\s*([\s\S]*?)\s*<\/CardGrid>/gu, "$1");
  output = output.replace(/^[ \t]*<Card\s+title=(['"])(.*?)\1\s*>\s*([\s\S]*?)\s*^[ \t]*<\/Card>/gmu, (_match, _quote, title, inner) => {
    const markdown = inner
      .split("\n")
      .map((line) => line.trim())
      .join("\n")
      .trim();
    return `### ${title}\n\n${markdown}\n`;
  });
  output = output.replace(/<br\s*\/?\s*>/giu, "\n");
  const mdxTag = output.match(/<\/?[A-Z][A-Za-z0-9]*(?:\s|>|\/)/u);
  if (mdxTag) {
    console.error(`warning: ${relPath}: possible unconverted MDX tag ${mdxTag[0].trim()}`);
  }
  return output;
}

function withoutMdExtension(value) {
  return value.replace(/\.(md|mdx)$/u, "");
}

function fileForDocRel(docRel) {
  const normalized = toPosix(path.posix.normalize(docRel));
  if (docPathToFile.has(normalized)) {
    return docPathToFile.get(normalized);
  }
  const indexMd = `${normalized}/index.md`;
  const indexMdx = `${normalized}/index.mdx`;
  if (docPathToFile.has(indexMd)) {
    return docPathToFile.get(indexMd);
  }
  if (docPathToFile.has(indexMdx)) {
    return docPathToFile.get(indexMdx);
  }
  return null;
}

function pageName(fileName) {
  return fileName.replace(/\.md$/u, "");
}

function rewriteLinkTarget(target, currentRelPath) {
  if (/^(?:[a-z][a-z0-9+.-]*:|#|mailto:)/iu.test(target)) {
    return target;
  }
  const [rawPath, suffix = ""] = target.split(/(?=[?#])/u, 2);
  if (!rawPath) {
    return target;
  }
  if (/\.(png|jpe?g|gif|svg|webp|avif)$/iu.test(rawPath)) {
    return rewriteImageTarget(rawPath) + suffix;
  }
  let wikiFile = null;
  if (rawPath.startsWith("/")) {
    const normalizedRoute = rawPath.endsWith("/") ? rawPath : `${rawPath}/`;
    wikiFile = routeToFile.get(normalizedRoute) || routeToFile.get(rawPath);
  } else {
    const currentDir = path.posix.dirname(currentRelPath);
    let docRel = withoutMdExtension(path.posix.normalize(path.posix.join(currentDir, rawPath)));
    if (rawPath.endsWith(".md") || rawPath.endsWith(".mdx")) {
      docRel = path.posix.normalize(path.posix.join(currentDir, rawPath));
    }
    wikiFile = fileForDocRel(docRel) || fileForDocRel(`${docRel}.md`) || fileForDocRel(`${docRel}.mdx`);
  }
  return wikiFile ? `${pageName(wikiFile)}${suffix}` : target;
}

function rewriteImageTarget(target) {
  const clean = target.split(/[?#]/u)[0];
  return `images/${path.posix.basename(clean)}`;
}

function rewriteLinksAndImages(text, relPath) {
  let output = text.replace(/(!\[[^\]]*\]\()([^\)]+)(\))/gu, (_match, prefix, target, suffix) => {
    return `${prefix}${rewriteImageTarget(target.trim())}${suffix}`;
  });
  output = output.replace(/(\[[^\]]+\]\()([^\)]+)(\))/gu, (_match, prefix, target, suffix) => {
    return `${prefix}${rewriteLinkTarget(target.trim(), relPath)}${suffix}`;
  });
  return output;
}

function convertFile(fullPath, relPath) {
  try {
    const original = fs.readFileSync(fullPath, "utf8");
    const { title, description, body } = parseFrontmatter(original, relPath);
    let converted = convertMdx(body, relPath).trim();
    converted = rewriteLinksAndImages(converted, relPath).trim();
    const parts = [`# ${title}`];
    if (description) {
      parts.push(`_${description}_`);
    }
    if (converted) {
      parts.push(converted);
    }
    return `${parts.join("\n\n")}\n`;
  } catch (error) {
    console.error(`warning: ${relPath}: ${error.message}`);
    return fs.existsSync(fullPath) ? fs.readFileSync(fullPath, "utf8") : "";
  }
}

function findLinkValue(lines, startIndex) {
  for (let index = startIndex; index < lines.length; index += 1) {
    const line = lines[index];
    if (index > startIndex && /^\s*\},?\s*$/u.test(line)) {
      return null;
    }
    const linkMatch = line.match(/link:\s*['"]([^'"]+)['"]/u);
    if (linkMatch) {
      return linkMatch[1];
    }
  }
  return null;
}

function parseSidebarItems(astroConfig) {
  const lines = astroConfig.split("\n");
  const roots = [];
  const stack = [{ indent: -1, items: roots }];
  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index];
    const inlineMatch = line.match(/^\s*\{\s*label:\s*['"]([^'"]+)['"],\s*link:\s*['"]([^'"]+)['"]/u);
    const labelMatch = inlineMatch || line.match(/^(\s*)(?:\{\s*)?label:\s*['"]([^'"]+)['"]/u);
    if (!labelMatch) {
      continue;
    }
    const indent = inlineMatch ? line.match(/^\s*/u)[0].length : labelMatch[1].length;
    while (stack.length > 1 && indent <= stack[stack.length - 1].indent) {
      stack.pop();
    }
    const item = { label: inlineMatch ? inlineMatch[1] : labelMatch[2], items: [] };
    item.link = inlineMatch ? inlineMatch[2] : findLinkValue(lines, index);
    stack[stack.length - 1].items.push(item);
    if (lines.slice(index, index + 8).some((candidate) => candidate.includes("items: ["))) {
      stack.push({ indent, items: item.items });
    }
  }
  return roots;
}

function renderSidebarItem(item, depth = 0) {
  const indent = "  ".repeat(depth);
  const bullet = depth === 0 ? "-" : "  -";
  const wikiFile = item.link ? routeToFile.get(item.link) || routeToFile.get(item.link.replace(/\/$/u, "")) : null;
  const label = wikiFile ? `[${item.label}](${pageName(wikiFile)})` : `**${item.label}**`;
  const lines = [`${indent}${bullet} ${label}`];
  for (const child of item.items || []) {
    lines.push(renderSidebarItem(child, depth + 1));
  }
  return lines.join("\n");
}

function writeSidebar() {
  const configPath = path.join(docsRoot, "astro.config.mjs");
  const config = fs.readFileSync(configPath, "utf8");
  const items = parseSidebarItems(config);
  const content = `${items.map((item) => renderSidebarItem(item)).join("\n")}\n`;
  fs.writeFileSync(path.join(outDir, "_Sidebar.md"), content);
}

fs.rmSync(outDir, { recursive: true, force: true });
fs.mkdirSync(outDir, { recursive: true });

for (const file of walk(srcDir)) {
  const relPath = toPosix(path.relative(srcDir, file));
  const outName = explicitMap.get(relPath) || deriveWikiFileName(relPath);
  docPathToFile.set(relPath, outName);
}

for (const [docPath, fileName] of docPathToFile) {
  if (docPath.startsWith("docs/")) {
    continue;
  }
  if (!explicitMap.has(docPath)) {
    const route = `/${docPath.replace(/\.(md|mdx)$/u, "")}/`;
    routeToFile.set(route, fileName);
    routeToFile.set(route.replace(/\/$/u, ""), fileName);
  }
}

for (const file of walk(srcDir)) {
  const relPath = toPosix(path.relative(srcDir, file));
  const outName = docPathToFile.get(relPath) || deriveWikiFileName(relPath);
  fs.writeFileSync(path.join(outDir, outName), convertFile(file, relPath));
}

const browserPath = path.join(docsRoot, "BROWSER_MODE.md");
if (fs.existsSync(browserPath)) {
  const relPath = "docs/BROWSER_MODE.md";
  const content = rewriteLinksAndImages(fs.readFileSync(browserPath, "utf8").trim(), relPath);
  fs.writeFileSync(path.join(outDir, "Browser-Automation.md"), `${content}\n`);
} else {
  console.error("warning: docs/BROWSER_MODE.md not found; skipping Browser-Automation.md");
}

writeSidebar();
fs.writeFileSync(path.join(outDir, "_Footer.md"), "Refact on GitHub: https://github.com/JegernOUTT/refact\n");
