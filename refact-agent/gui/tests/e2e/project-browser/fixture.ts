import { chromium, type BrowserContext } from "@playwright/test";
import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import { randomBytes, randomUUID } from "node:crypto";
import { readFileSync, readlinkSync } from "node:fs";
import { access, mkdir, readFile, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import { isAbsolute, join } from "node:path";
import { setTimeout as delay } from "node:timers/promises";

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

async function poll<T>(
  read: () => Promise<T | undefined>,
  name: string,
): Promise<T> {
  const end = Date.now() + 20_000;
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

export async function createProjectRoot(
  fixture: ReturnType<typeof fixtureEnvironment>,
  name: string,
) {
  const root = join(fixture.scratch, name);
  await mkdir(root);
  execFileSync("git", ["init", "--quiet"], { cwd: root, env: fixture.env });
  return root;
}

export class Daemon {
  readonly token = randomBytes(32).toString("hex");
  readonly dir: string;
  private child?: ChildProcess;
  private exited?: Promise<{
    code: number | null;
    signal: NodeJS.Signals | null;
  }>;
  private daemonOutput = "";
  origin = "";

  constructor(private readonly fixture: ReturnType<typeof fixtureEnvironment>) {
    this.dir = join(fixture.scratch, "daemon");
  }

  async start(siteOrigin: string) {
    const engineConfig = join(this.fixture.home, ".config", "refact");
    const providers = join(engineConfig, "providers.d");
    await mkdir(providers, { recursive: true });
    await writeFile(
      join(providers, "custom.yaml"),
      JSON.stringify({
        enabled: true,
        chat_endpoint: `${siteOrigin}/model-must-not-be-invoked`,
        enabled_models: ["fixture-model"],
        custom_models: { "fixture-model": { n_ctx: 4096, tokenizer: "fake" } },
      }),
    );
    await writeFile(
      join(engineConfig, "browser.yaml"),
      JSON.stringify({
        launch: {
          chrome_path: this.fixture.chrome,
          chromium_sandbox: false,
          extra_args: ["--password-store=basic"],
        },
      }),
    );
    await mkdir(this.dir, { recursive: true, mode: 0o700 });
    await writeFile(
      join(this.dir, "daemon.yaml"),
      JSON.stringify({
        bind: "127.0.0.1",
        auth: { enabled: true, token: this.token },
        mdns: { enabled: false },
      }),
      { mode: 0o600 },
    );
    const { engine, chrome, env, scratch } = this.fixture;
    const child = spawn(engine, ["daemon", "--foreground", "--port", "0"], {
      cwd: scratch,
      env: { ...env, CHROME: chrome, REFACT_DAEMON_DIR: this.dir },
      stdio: ["ignore", "pipe", "pipe"],
    });
    this.child = child;
    for (const pipe of [child.stdout, child.stderr]) {
      pipe?.on("data", (chunk: Buffer) => {
        this.daemonOutput = (this.daemonOutput + chunk.toString()).slice(
          -32_768,
        );
      });
    }
    this.exited = new Promise((resolve, reject) => {
      child.on("error", reject);
      child.on("exit", (code, signal) => resolve({ code, signal }));
    });
    void this.exited.catch(() => undefined);
    const port = await Promise.race([
      this.exited.then(() => {
        throw new Error("Disposable daemon exited before readiness");
      }),
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
        Authorization: `Bearer ${this.token}`,
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

  async installProjectCookie(context: BrowserContext, projectId: string) {
    const path = `/p/${projectId}/`;
    const response = await fetch(
      `${this.origin}${path}?daemon_token=${this.token}`,
      {
        redirect: "manual",
        signal: AbortSignal.timeout(20_000),
      },
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
    await context.addCookies([
      {
        domain: "127.0.0.1",
        path,
        name: pair[1],
        value: pair[2],
        httpOnly: true,
        sameSite: "Strict",
      },
    ]);
    return `${this.origin}${path}`;
  }

  async stop() {
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
      throw new Error("Disposable daemon did not shut down gracefully");
    }
    if (outcome.code !== 0 || outcome.signal !== null) {
      throw new Error(
        `Disposable daemon did not exit cleanly (${
          outcome.code ?? outcome.signal
        })`,
      );
    }
  }

  diagnostics() {
    return this.daemonOutput.replaceAll(this.token, "[fixture token]");
  }
}

const styles = `body{margin:0;font:20px system-ui;color:white;background:rgb(20,30,40)}
main{padding:12px 16px;min-height:950px;box-sizing:border-box}h1{margin:0 0 22px}
form{position:absolute;left:16px;top:70px;display:flex;align-items:center;gap:12px}input{box-sizing:border-box;width:150px;height:34px}
button{width:75px;height:34px}footer{height:900px;padding:15px;background:rgb(10,140,110);box-sizing:border-box}`;

export async function startSite() {
  const visits: {
    path: string;
    value: string | null;
    cookie: string;
    userAgent: string;
  }[] = [];
  const server = createServer((req, res) => {
    const url = new URL(req.url ?? "/", "http://127.0.0.1");
    const { pathname: path } = url;
    if (path === "/favicon.ico") {
      res.writeHead(204).end();
      return;
    }
    const cookie = req.headers.cookie ?? "";
    const value = url.searchParams.get("entry");
    visits.push({
      path,
      value,
      cookie,
      userAgent: req.headers["user-agent"] ?? "",
    });
    if (path === "/model-must-not-be-invoked") {
      res.writeHead(501).end("This test does not invoke a model");
      return;
    }
    const login = cookie.split("; ").includes("fixture_login=project-alpha");
    const pageTitle =
      path === "/check"
        ? login
          ? "Signed in: Project Alpha"
          : "Signed out"
        : path === "/submit"
          ? "Browser fixture · submitted"
          : "Browser fixture · form";
    res.setHeader("Content-Type", "text/html; charset=utf-8");
    res.setHeader("Cache-Control", "no-store");
    res.setHeader(
      "Content-Security-Policy",
      "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; frame-ancestors 'none'",
    );
    if (path === "/submit")
      res.setHeader(
        "Set-Cookie",
        "fixture_login=project-alpha; Max-Age=86400; Path=/; HttpOnly; SameSite=Strict",
      );
    if (path === "/seed-selected" || path === "/seed-unselected") {
      res.setHeader(
        "Set-Cookie",
        `fixture_legacy=${path.slice(
          6,
        )}; Max-Age=86400; Path=/; HttpOnly; SameSite=Strict`,
      );
    }
    res.end(`<!doctype html><html lang="en"><meta charset="utf-8"><title>${pageTitle}</title><style>${styles}</style><main><h1>${pageTitle}</h1>
<form action="/submit"><input name="entry" aria-label="Fixture entry"><button type="submit">Submit</button></form></main><footer>Browser fixture · scrolled</footer></html>`);
  });
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const address = server.address();
  if (!address || typeof address === "string")
    throw new Error("Missing fixture address");
  return {
    origin: `http://127.0.0.1:${address.port}`,
    visits,
    close: () =>
      new Promise<void>((resolve, reject) =>
        server.close((error) => (error ? reject(error) : resolve())),
      ),
  };
}

export async function seedChat(
  daemon: Daemon,
  project: Project,
  title: string,
) {
  const id = randomUUID();
  const now = new Date().toISOString();
  await daemon.request(
    `/p/${project.project_id}/v1/trajectories/${id}`,
    {
      id,
      title,
      created_at: now,
      updated_at: now,
      model: "",
      mode: "agent",
      tool_use: "agent",
      messages: [{ role: "user", content: `${title} fixture message` }],
    },
    "PUT",
  );
  return id;
}

export async function seedLegacyProfile(
  fixture: ReturnType<typeof fixtureEnvironment>,
  project: Project,
  siteOrigin: string,
  profile: "selected" | "unselected",
) {
  const id = `fixture-legacy-${profile}`;
  const cache = join(fixture.home, ".cache", "refact", "browser_profiles", id);
  await mkdir(cache, { recursive: true });
  const context = await chromium.launchPersistentContext(cache, {
    executablePath: fixture.chrome,
    env: fixture.env,
    args: ["--password-store=basic"],
  });
  try {
    await context.pages()[0].goto(`${siteOrigin}/seed-${profile}`);
  } finally {
    await context.close();
  }
  const trajectories = join(project.root, ".refact", "trajectories");
  await mkdir(trajectories, { recursive: true });
  const now = new Date().toISOString();
  await writeFile(
    join(trajectories, `${id}.json`),
    JSON.stringify({
      id,
      title: `Legacy ${profile}`,
      created_at: now,
      updated_at: now,
      model: "",
      mode: "agent",
      tool_use: "agent",
      messages: [],
    }),
  );
  return { id, path: cache };
}
