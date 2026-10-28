export interface ToolCallTitle {
  verb: string;
  target?: string;
  meta?: string;
}

const MCP_SERVERS: Partial<Record<string, string>> = {
  playwright: "Playwright",
  github: "GitHub",
  gitlab: "GitLab",
  postgres: "Postgres",
  mysql: "MySQL",
  sqlite: "SQLite",
  fetch: "Fetch",
  filesystem: "Filesystem",
  slack: "Slack",
  sentry: "Sentry",
  jira: "Jira",
  linear: "Linear",
  notion: "Notion",
  docker: "Docker",
  kubernetes: "Kubernetes",
  context7: "Context7",
  memory: "Memory",
  puppeteer: "Puppeteer",
  brave: "Brave",
  google: "Google",
  aws: "AWS",
};

const SERVER_REDUNDANT_PREFIX: Partial<Record<string, string>> = {
  playwright: "browser_",
  puppeteer: "puppeteer_",
};

const NOISE_WORDS = new Set(["unsafe"]);

const ACRONYMS: Partial<Record<string, string>> = {
  mcp: "MCP",
  url: "URL",
  sql: "SQL",
  api: "API",
  id: "ID",
  pr: "PR",
  ui: "UI",
  js: "JS",
  ide: "IDE",
};

const PRIMARY_KEYS = [
  "url",
  "query",
  "label",
  "path",
  "file_path",
  "filename",
  "name",
  "title",
  "repo",
] as const;

const PRIMARY_MAX_CHARS = 80;
const TITLE_MAX_CHARS = 120;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function clip(text: string, max: number): string {
  const normalized = text.replace(/\s+/g, " ").trim();
  if (normalized.length <= max) return normalized;
  return `${normalized.slice(0, max - 1)}…`;
}

function words(name: string): string[] {
  return name
    .split(/[_\s-]+/)
    .filter(Boolean)
    .filter((part) => !NOISE_WORDS.has(part.toLowerCase()));
}

export function humanizeToolName(name: string): string {
  const parts = words(name);
  if (parts.length === 0) return "Tool";
  return parts
    .map((part, index) => {
      const lower = part.toLowerCase();
      const acronym = ACRONYMS[lower];
      if (acronym) return acronym;
      return index === 0 ? lower[0].toUpperCase() + lower.slice(1) : lower;
    })
    .join(" ");
}

export function parseToolArgs(args: unknown): Record<string, unknown> {
  if (isRecord(args)) return args;
  if (typeof args !== "string" || args.trim() === "") return {};
  try {
    const parsed: unknown = JSON.parse(args);
    return isRecord(parsed) ? parsed : {};
  } catch {
    return {};
  }
}

function isShortString(value: unknown): value is string {
  return (
    typeof value === "string" &&
    value.trim().length > 0 &&
    !value.includes("\n") &&
    value.length <= TITLE_MAX_CHARS
  );
}

export function primaryArgument(
  args: Record<string, unknown>,
): string | undefined {
  for (const key of PRIMARY_KEYS) {
    const value = args[key];
    if (typeof value === "string" && value.trim().length > 0)
      return clip(value, PRIMARY_MAX_CHARS);
  }
  const first = Object.values(args).find(isShortString);
  return first ? clip(first, PRIMARY_MAX_CHARS) : undefined;
}

function splitMcpName(name: string): { server: string; action: string } | null {
  const stripped = name.startsWith("mcp_") ? name.slice(4) : name;
  const [head, ...rest] = stripped.split("_");
  if (rest.length === 0) return null;
  const known = MCP_SERVERS[head];
  if (!known && stripped === name) return null;
  const server = known ?? humanizeToolName(head);
  let action = rest.join("_");
  const redundant = SERVER_REDUNDANT_PREFIX[head];
  if (redundant && action.startsWith(redundant) && action !== redundant)
    action = action.slice(redundant.length);
  return { server, action: humanizeToolName(action) };
}

export function describeToolCall(
  name: string | undefined,
  rawArgs: unknown,
): ToolCallTitle {
  const toolName = (name ?? "").trim() || "tool";
  const args = parseToolArgs(rawArgs);

  if (toolName === "set_trajectory_label") {
    const label = typeof args.label === "string" ? args.label : undefined;
    return { verb: "Label", target: label && clip(label, TITLE_MAX_CHARS) };
  }
  if (toolName === "mcp_tool_search") {
    const query = typeof args.query === "string" ? args.query : undefined;
    return { verb: "MCP tools", target: query && clip(query, TITLE_MAX_CHARS) };
  }
  if (toolName === "mcp_call") {
    const inner = typeof args.tool_name === "string" ? args.tool_name : "";
    const split = inner ? splitMcpName(`mcp_${inner}`) : null;
    const innerArgs = parseToolArgs(args.args);
    return {
      verb: split?.server ?? "MCP",
      target: split?.action ?? (inner ? humanizeToolName(inner) : undefined),
      meta: primaryArgument(innerArgs),
    };
  }

  const split = splitMcpName(toolName);
  if (split) {
    return {
      verb: split.server,
      target: split.action,
      meta: primaryArgument(args),
    };
  }
  return { verb: humanizeToolName(toolName), meta: primaryArgument(args) };
}
