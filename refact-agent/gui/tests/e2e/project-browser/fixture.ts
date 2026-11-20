import { chromium } from "@playwright/test";
import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import { randomBytes, randomUUID } from "node:crypto";
import { readFileSync, readlinkSync } from "node:fs";
import { access, mkdir, readFile, utimes, writeFile } from "node:fs/promises";
import { createServer, type IncomingMessage } from "node:http";
import { createServer as createNetServer } from "node:net";
import { isAbsolute, join } from "node:path";
import { setTimeout as delay } from "node:timers/promises";

export const FIXTURE_MODEL = "custom/fixture-model";
export const NON_LOOPBACK_HOST = "192.0.2.1";

function required(name: string): string {
  const path = process.env[name];
  if (!path || !isAbsolute(path)) throw new Error(`Missing absolute ${name}`);
  return path;
}

export function assertIsolated() {
  const hostNet = process.env.REFACT_PROJECT_BROWSER_HOST_NET;
  const interfaces = readFileSync("/proc/net/dev", "utf8")
    .split("\n")
    .flatMap((line) => (line.includes(":") ? [line.split(":")[0].trim()] : []));
  if (
    !hostNet ||
    readlinkSync("/proc/self/ns/net") === hostNet ||
    interfaces.sort().join(",") !== "fixture0,lo" ||
    execFileSync("ip", ["-details", "link", "show", "fixture0"], {
      encoding: "utf8",
    }).includes("\n    dummy ") === false ||
    execFileSync("ip", ["-4", "route", "show", "table", "main"], {
      encoding: "utf8",
    }).trim() !== "" ||
    execFileSync("ip", ["-6", "route", "show", "table", "main"], {
      encoding: "utf8",
    }).trim() !== ""
  ) {
    throw new Error(
      "Project Browser acceptance requires a fresh unrouted namespace with only loopback and a dummy carrier",
    );
  }
}

export type Fixture = ReturnType<typeof fixtureEnvironment>;

export function fixtureEnvironment() {
  assertIsolated();
  const scratch = required("REFACT_PROJECT_BROWSER_SCRATCH");
  const home = required("HOME");
  if (home !== join(scratch, "home"))
    throw new Error("Fixture HOME is not isolated");
  for (const [key, dir] of Object.entries({
    XDG_CACHE_HOME: "cache",
    XDG_CONFIG_HOME: "config",
    XDG_DATA_HOME: "data",
    XDG_RUNTIME_DIR: "runtime",
  })) {
    if (required(key) !== join(scratch, dir))
      throw new Error(`Fixture ${key} is not isolated`);
  }
  return {
    scratch,
    home,
    engine: required("REFACT_PROJECT_BROWSER_ENGINE"),
    chrome: required("REFACT_PROJECT_BROWSER_CHROME"),
    cache: join(home, ".cache", "refact"),
    env: {
      PATH: "/usr/bin:/bin:/usr/sbin",
      HOME: home,
      XDG_CACHE_HOME: join(scratch, "cache"),
      XDG_CONFIG_HOME: join(scratch, "config"),
      XDG_DATA_HOME: join(scratch, "data"),
      XDG_RUNTIME_DIR: join(scratch, "runtime"),
      TMPDIR: "/tmp",
    },
  };
}

export async function poll<T>(
  read: () => Promise<T | undefined>,
  name: string,
  timeout = 30_000,
): Promise<T> {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    const value = await read();
    if (value !== undefined) return value;
    await delay(100);
  }
  throw new Error(`Timed out waiting for ${name}`);
}

export type Project = {
  project_id: string;
  root: string;
  worker: { pid: number; http_port: number };
};

export async function createProjectRoot(fixture: Fixture, name: string) {
  const root = join(fixture.scratch, name);
  await mkdir(root);
  execFileSync("git", ["init", "--quiet"], { cwd: root, env: fixture.env });
  return root;
}

export async function writeEngineConfig(fixture: Fixture, siteOrigin: string) {
  const engineConfig = join(fixture.home, ".config", "refact");
  const providers = join(engineConfig, "providers.d");
  await mkdir(providers, { recursive: true });
  await writeFile(
    join(providers, "custom.yaml"),
    JSON.stringify({
      enabled: true,
      chat_endpoint: `${siteOrigin}/v1/chat/completions`,
      wire_format: "openai_chat_completions",
      enabled_models: ["fixture-model"],
      custom_models: {
        "fixture-model": {
          n_ctx: 400_000,
          supports_tools: true,
          tokenizer: "fake",
          max_output_tokens: 4096,
        },
      },
    }),
  );
  await writeFile(
    join(engineConfig, "browser.yaml"),
    JSON.stringify({
      launch: {
        chrome_path: fixture.chrome,
        chromium_sandbox: false,
        extra_args: ["--password-store=basic"],
      },
    }),
  );
}

async function freePort(): Promise<number> {
  const server = createNetServer();
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "0.0.0.0", resolve);
  });
  const address = server.address();
  await new Promise<void>((resolve) => server.close(() => resolve()));
  if (!address || typeof address === "string")
    throw new Error("No free port for the worker");
  return address.port;
}

class OwnedProcess {
  protected child?: ChildProcess;
  protected exited?: Promise<{
    code: number | null;
    signal: NodeJS.Signals | null;
  }>;
  protected output = "";

  protected spawnOwned(
    command: string,
    args: string[],
    cwd: string,
    env: NodeJS.ProcessEnv,
  ) {
    const child = spawn(command, args, {
      cwd,
      env,
      stdio: ["ignore", "pipe", "pipe"],
    });
    this.child = child;
    for (const pipe of [child.stdout, child.stderr]) {
      pipe?.on("data", (chunk: Buffer) => {
        this.output = (this.output + chunk.toString()).slice(-16_777_216);
      });
    }
    this.exited = new Promise((resolve, reject) => {
      child.on("error", reject);
      child.on("exit", (code, signal) => resolve({ code, signal }));
    });
    void this.exited.catch(() => undefined);
    return child;
  }

  protected exitedEarly(label: string) {
    if (!this.exited) throw new Error(`${label} was not started`);
    return this.exited.then(() => {
      throw new Error(`${label} exited before readiness`);
    });
  }

  get pid() {
    return this.child?.pid;
  }

  async stop(label: string) {
    const child = this.child;
    const exited = this.exited;
    if (
      !child ||
      !exited ||
      child.exitCode !== null ||
      child.signalCode !== null
    )
      return;
    child.kill("SIGTERM");
    const outcome = await Promise.race([
      exited,
      delay(35_000, undefined, { ref: false }),
    ]);
    if (!outcome) {
      child.kill("SIGKILL");
      await exited;
      throw new Error(`${label} did not shut down gracefully`);
    }
    if (outcome.code !== 0 && outcome.signal !== "SIGTERM") {
      throw new Error(
        `${label} did not exit cleanly (${outcome.code ?? outcome.signal})`,
      );
    }
  }
}

/** The user's setup: one standalone worker for one project, no daemon. */
export class Worker extends OwnedProcess {
  origin = "";
  port = 0;

  constructor(
    private readonly fixture: Fixture,
    readonly root: string,
  ) {
    super();
  }

  async start(port?: number) {
    this.port = port ?? (await freePort());
    this.output += `\n===== worker start on port ${
      this.port
    } at ${new Date().toISOString()} =====\n`;
    this.spawnOwned(
      this.fixture.engine,
      [
        "worker",
        "--http-port",
        String(this.port),
        "--http-host",
        "0.0.0.0",
        "--workspace-folder",
        this.root,
        "--no-scheduler",
        "--logs-stderr",
      ],
      this.root,
      this.fixture.env,
    );
    this.origin = `http://${NON_LOOPBACK_HOST}:${this.port}`;
    await Promise.race([
      this.exitedEarly("Standalone worker"),
      poll(
        async () => {
          try {
            const response = await fetch(`${this.origin}/v1/ping`, {
              signal: AbortSignal.timeout(2_000),
            });
            return response.ok ? true : undefined;
          } catch {
            return undefined;
          }
        },
        "standalone worker readiness on the non-loopback address",
        90_000,
      ),
    ]);
  }

  async request(
    path: string,
    body?: object,
    method = body === undefined ? "GET" : "POST",
  ) {
    const response = await fetch(`${this.origin}${path}`, {
      method,
      headers: { "Content-Type": "application/json" },
      body: body === undefined ? undefined : JSON.stringify(body),
      redirect: "error",
      signal: AbortSignal.timeout(125_000),
    });
    if (!response.ok)
      throw new Error(
        `${path}: HTTP ${response.status}: ${await response.text()}`,
      );
    return response;
  }

  stopWorker() {
    return this.stop("Standalone worker");
  }

  diagnostics() {
    return this.output;
  }
}

export class Daemon extends OwnedProcess {
  readonly token = randomBytes(32).toString("hex");
  readonly dir: string;
  private authEnabled = true;
  origin = "";

  constructor(
    private readonly fixture: Fixture,
    name: string,
  ) {
    super();
    this.dir = join(fixture.scratch, name);
  }

  async start(options: { authEnabled: boolean }) {
    this.authEnabled = options.authEnabled;
    await mkdir(this.dir, { recursive: true, mode: 0o700 });
    await writeFile(
      join(this.dir, "daemon.yaml"),
      JSON.stringify({
        bind: "127.0.0.1",
        auth: {
          enabled: this.authEnabled,
          ...(this.authEnabled ? { token: this.token } : {}),
        },
        mdns: { enabled: false },
      }),
      { mode: 0o600 },
    );
    const child = this.spawnOwned(
      this.fixture.engine,
      ["daemon", "--foreground", "--port", "0"],
      this.fixture.scratch,
      { ...this.fixture.env, REFACT_DAEMON_DIR: this.dir },
    );
    const port = await Promise.race([
      this.exitedEarly("Disposable daemon"),
      poll(async () => {
        const path = join(this.dir, "daemon.json");
        try {
          await access(path);
        } catch (error) {
          if ((error as NodeJS.ErrnoException).code === "ENOENT")
            return undefined;
          throw error;
        }
        const info = JSON.parse(await readFile(path, "utf8")) as {
          pid: number;
          port: number;
        };
        return info.pid === child.pid && info.port > 0 ? info.port : undefined;
      }, "owned daemon PID and port"),
    ]);
    this.origin = `http://127.0.0.1:${port}`;
    await this.request("/daemon/v1/status");
  }

  async request(
    path: string,
    body?: object,
    method = body === undefined ? "GET" : "POST",
  ) {
    const response = await fetch(`${this.origin}${path}`, {
      method,
      headers: {
        ...(this.authEnabled ? { Authorization: `Bearer ${this.token}` } : {}),
        "Content-Type": "application/json",
      },
      body: body === undefined ? undefined : JSON.stringify(body),
      redirect: "error",
      signal: AbortSignal.timeout(125_000),
    });
    if (!response.ok) {
      const detail = (await response.text()).replaceAll(
        this.token,
        "[fixture token]",
      );
      throw new Error(`${path}: HTTP ${response.status}: ${detail}`);
    }
    return response;
  }

  async openProject(root: string) {
    return (
      await this.request("/daemon/v1/projects/open", { root })
    ).json() as Promise<Project>;
  }

  /** The normal UI login: the daemon token bootstrap that issues the scoped project cookie. */
  async loginCookie(projectId: string) {
    const path = `/p/${projectId}/`;
    const response = await fetch(
      `${this.origin}${path}?daemon_token=${this.token}`,
      { redirect: "manual", signal: AbortSignal.timeout(20_000) },
    );
    if (response.status !== 303 || response.headers.get("location") !== path) {
      throw new Error(
        `Daemon project-cookie bootstrap returned HTTP ${response.status}`,
      );
    }
    const cookie = response.headers
      .getSetCookie()
      .find((header) => header.includes(`Path=${path}`));
    const pair = cookie?.match(/^([^=]+)=([^;]+); HttpOnly; SameSite=Strict; /);
    if (!pair)
      throw new Error("Daemon did not issue an HttpOnly scoped project cookie");
    return {
      domain: "127.0.0.1",
      path,
      name: pair[1],
      value: pair[2],
      httpOnly: true,
      sameSite: "Strict" as const,
    };
  }

  stopDaemon() {
    return this.stop("Disposable daemon");
  }

  diagnostics() {
    return this.output.replaceAll(this.token, "[fixture token]");
  }
}

const styles = `body{margin:0;font:20px system-ui;color:white;background:rgb(20,30,40)}
main{padding:12px 16px;min-height:950px;box-sizing:border-box}h1{margin:0 0 22px;font-size:24px}
form{position:absolute;left:16px;top:70px;display:flex;align-items:center;gap:12px}input{box-sizing:border-box;width:150px;height:34px}
button{width:75px;height:34px}`;

export type Visit = {
  at: number;
  path: string;
  user: string | null;
  cookie: string;
  userAgent: string;
};

export type LlmCall = {
  script: string | null;
  responded: "tool_call" | "final" | "text";
  toolCallId?: string;
  roles: string[];
  users: string[];
  tools: number;
  hasChrome: boolean;
  at: number;
  /** Text of the tool result the model received on this turn, if any. */
  toolResult?: string;
  resultCallId?: string;
};

type ChatMessage = {
  role: string;
  content?: unknown;
  tool_calls?: unknown[];
  tool_call_id?: string;
};

function messageText(content: unknown): string {
  if (typeof content === "string") return content;
  if (Array.isArray(content))
    return content
      .map((part) =>
        typeof part === "object" && part && "text" in part
          ? String((part as { text: unknown }).text)
          : "",
      )
      .join("\n");
  return "";
}

/** Only a whole typed user message selects a script, never context that quotes one. */
const SCRIPT =
  /^\s*(?:Please browse|Just answer, no browsing): fixture script ([a-z0-9-]+)\s*$/m;

async function readBody(req: IncomingMessage) {
  const chunks: Buffer[] = [];
  for await (const chunk of req) chunks.push(chunk as Buffer);
  return Buffer.concat(chunks).toString("utf8");
}

/**
 * Local site plus an OpenAI-compatible streaming model. A user message that says
 * `fixture script <key>` makes the model call `chrome` once with the registered request;
 * the next model turn after that tool call answers with a short final message.
 */
export async function startSite() {
  const visits: Visit[] = [];
  const llmCalls: LlmCall[] = [];
  const scripts = new Map<string, object>();
  let callCounter = 0;

  const completion = (body: string) => {
    const payload = JSON.parse(body) as {
      messages?: ChatMessage[];
      tools?: { function?: { name?: string } }[];
      stream?: boolean;
      model?: string;
    };
    const messages = payload.messages ?? [];
    const hasChrome = (payload.tools ?? []).some(
      (tool) => tool.function?.name === "chrome",
    );
    let asked = -1;
    let script: string | null = null;
    for (const [index, message] of messages.entries()) {
      const match =
        message.role === "user"
          ? SCRIPT.exec(messageText(message.content))
          : null;
      if (!match) continue;
      asked = index;
      script = match[1];
    }
    const answered =
      asked >= 0 &&
      messages
        .slice(asked + 1)
        .some(
          (message) =>
            message.role === "tool" ||
            (message.role === "assistant" &&
              (message.tool_calls?.length ?? 0) > 0),
        );
    const request = script ? scripts.get(script) : undefined;
    const waitResult =
      script === "waitlogin" && asked >= 0
        ? messages.slice(asked + 1).find((message) => message.role === "tool")
        : undefined;
    const seen = {
      roles: messages.map((message) => message.role),
      users: messages
        .filter((message) => message.role === "user")
        .map((message) => messageText(message.content).slice(0, 160)),
      tools: payload.tools?.length ?? 0,
      hasChrome,
      at: Date.now(),
      resultCallId: waitResult?.tool_call_id,
      toolResult: waitResult
        ? messageText(waitResult.content).slice(0, 20_000)
        : undefined,
    };
    let delta: Record<string, unknown>;
    let finish: string;
    if (request && hasChrome && !answered) {
      callCounter += 1;
      const id = `call_fixture_${callCounter}_${randomUUID().slice(0, 8)}`;
      llmCalls.push({
        script,
        responded: "tool_call",
        toolCallId: id,
        ...seen,
      });
      delta = {
        role: "assistant",
        content: null,
        tool_calls: [
          {
            index: 0,
            id,
            type: "function",
            function: {
              name: "chrome",
              arguments: JSON.stringify({ request }),
            },
          },
        ],
      };
      finish = "tool_calls";
    } else {
      llmCalls.push({
        script,
        responded: answered ? "final" : "text",
        ...seen,
      });
      delta = {
        role: "assistant",
        content: answered
          ? `Fixture browsing done (${script ?? "no script"}).`
          : `Fixture model reply (${script ?? "no script"}).`,
      };
      finish = "stop";
    }
    const base = {
      id: `fixture-${randomUUID()}`,
      created: Math.floor(Date.now() / 1000),
      model: payload.model ?? "fixture-model",
    };
    if (!payload.stream) {
      return {
        type: "application/json",
        text: JSON.stringify({
          ...base,
          object: "chat.completion",
          choices: [{ index: 0, finish_reason: finish, message: delta }],
          usage: { prompt_tokens: 10, completion_tokens: 5, total_tokens: 15 },
        }),
      };
    }
    const chunk = (choice: object, extra: object = {}) =>
      `data: ${JSON.stringify({
        ...base,
        object: "chat.completion.chunk",
        choices: [choice],
        ...extra,
      })}\n\n`;
    return {
      type: "text/event-stream",
      text:
        chunk({ index: 0, delta, finish_reason: null }) +
        chunk(
          { index: 0, delta: {}, finish_reason: finish },
          {
            usage: {
              prompt_tokens: 10,
              completion_tokens: 5,
              total_tokens: 15,
            },
          },
        ) +
        "data: [DONE]\n\n",
    };
  };

  const server = createServer((req, res) => {
    const url = new URL(req.url ?? "/", "http://127.0.0.1");
    const { pathname: path } = url;
    if (path === "/favicon.ico") {
      res.writeHead(204).end();
      return;
    }
    if (path === "/v1/models") {
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(
        JSON.stringify({
          object: "list",
          data: [{ id: "fixture-model", object: "model" }],
        }),
      );
      return;
    }
    if (path === "/v1/chat/completions" && req.method === "POST") {
      void readBody(req)
        .then((body) => {
          const reply = completion(body);
          res.writeHead(200, {
            "Content-Type": reply.type,
            "Cache-Control": "no-store",
          });
          res.end(reply.text);
        })
        .catch((error: unknown) => {
          res.writeHead(500).end(String(error));
        });
      return;
    }
    const cookie = req.headers.cookie ?? "";
    const user = url.searchParams.get("user");
    visits.push({
      at: Date.now(),
      path,
      user,
      cookie,
      userAgent: req.headers["user-agent"] ?? "",
    });
    const signedIn = cookie
      .split("; ")
      .find((pair) => pair.startsWith("fixture_login="))
      ?.slice("fixture_login=".length);
    const pageTitle =
      path === "/signin" && user
        ? `Signed in as ${user}`
        : path === "/check"
          ? signedIn
            ? `Signed in as ${signedIn}`
            : "Signed out"
          : path === "/login"
            ? "Fixture login"
            : `Fixture page ${path}`;
    res.setHeader("Content-Type", "text/html; charset=utf-8");
    res.setHeader("Cache-Control", "no-store");
    res.setHeader(
      "Content-Security-Policy",
      `default-src 'none'; ${
        (path === "/login" && url.searchParams.has("wait")) ||
        path === "/cadence"
          ? "connect-src 'self';"
          : ""
      } ${
        path === "/cadence" ? "script-src 'unsafe-inline';" : ""
      } style-src 'unsafe-inline'; form-action 'self'; frame-ancestors 'none'`,
    );
    if (path === "/signin" && user) {
      res.setHeader(
        "Set-Cookie",
        `fixture_login=${encodeURIComponent(
          user,
        )}; Max-Age=86400; Path=/; HttpOnly; SameSite=Lax`,
      );
      const next = url.searchParams.get("next");
      if (next === "/check") {
        res.writeHead(303, { Location: "/check" }).end();
        return;
      }
    }
    const legacy = /^\/seed-(older|newer)$/.exec(path)?.[1];
    if (legacy) {
      res.setHeader(
        "Set-Cookie",
        `fixture_legacy=${legacy}; Max-Age=86400; Path=/; HttpOnly; SameSite=Lax`,
      );
    }
    if (path === "/cadence") {
      res.end(`<!doctype html><html><meta charset="utf-8"><title>Cadence fixture</title>
        <style>body{margin:0;background:#18304c}button{position:fixed;top:20px;left:20px;width:200px;height:80px;font-size:24px;z-index:2;background:#b02030}#marker{position:fixed;top:120px;left:30px;width:180px;height:120px;background:#d02030}.running button{background:#20d050}.running #marker{animation:move .7s linear infinite alternate}@keyframes move{to{transform:translateX(220px);background:#20e050}}#stripes{height:4000px;background:linear-gradient(to bottom,#193858 0 470px,#e8d048 470px 4000px)}</style>
        <button onclick="document.body.classList.add('running');this.textContent='Running'">Start animation</button><div id="marker"></div><div id="stripes"></div>
        <script>let reported=false;addEventListener('scroll',()=>{if(!reported){reported=true;fetch('/cadence-scrolled')}})</script></html>`);
      return;
    }
    const form =
      path === "/login"
        ? `<form action="/signin"><input name="user" aria-label="User">${
            url.searchParams.has("wait")
              ? `<input type="hidden" name="next" value="/check">`
              : ""
          }<button type="submit">Sign in</button></form>`
        : "";
    res.end(
      `<!doctype html><html lang="en"><meta charset="utf-8"><title>${pageTitle}</title><style>${styles}</style><main><h1>${pageTitle}</h1>${form}</main></html>`,
    );
  });
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const address = server.address();
  if (!address || typeof address === "string")
    throw new Error("Missing fixture address");
  const origin = `http://127.0.0.1:${address.port}`;
  return {
    origin,
    visits,
    llmCalls,
    /** Registers the chrome request the model issues for `fixture script <key>`. */
    script(key: string, request: object) {
      scripts.set(key, request);
      return `Please browse: fixture script ${key}`;
    },
    lastVisit(path: string, since = 0) {
      return visits
        .slice(since)
        .filter((visit) => visit.path === path)
        .at(-1);
    },
    close: () =>
      new Promise<void>((resolve, reject) => {
        server.closeAllConnections();
        server.close((error) => (error ? reject(error) : resolve()));
      }),
  };
}

export type Site = Awaited<ReturnType<typeof startSite>>;

export function trajectory(id: string, title: string) {
  const now = new Date().toISOString();
  return {
    id,
    title,
    created_at: now,
    updated_at: now,
    model: FIXTURE_MODEL,
    mode: "agent",
    tool_use: "agent",
    messages: [],
  };
}

/** Writes a chat trajectory directly into the project's storage (before a worker starts). */
export async function writeChat(
  root: string,
  title: string,
  id: string = randomUUID(),
) {
  const dir = join(root, ".refact", "trajectories");
  await mkdir(dir, { recursive: true });
  await writeFile(
    join(dir, `${id}.json`),
    JSON.stringify(trajectory(id, title)),
  );
  return id;
}

/**
 * A pre-upgrade per-chat profile `<cache>/browser_profiles/<chat_id>` of a chat in this
 * project, carrying a synthetic cookie, with a controlled last-use time.
 */
export async function seedLegacyProfile(
  fixture: Fixture,
  root: string,
  site: Site,
  which: "older" | "newer",
  modified: Date,
) {
  const id = `legacy-${which}-chat`;
  const path = join(fixture.cache, "browser_profiles", id);
  await mkdir(path, { recursive: true });
  const context = await chromium.launchPersistentContext(path, {
    executablePath: fixture.chrome,
    env: fixture.env,
    args: ["--password-store=basic"],
  });
  try {
    await context.pages()[0].goto(`${site.origin}/seed-${which}`);
  } finally {
    await context.close();
  }
  await writeChat(root, `Legacy ${which} chat`, id);
  await utimes(path, modified, modified);
  return { id, path };
}
