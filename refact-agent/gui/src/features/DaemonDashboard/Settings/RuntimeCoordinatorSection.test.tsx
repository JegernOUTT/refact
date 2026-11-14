import { http, HttpResponse } from "msw";
import { beforeEach, describe, expect, it } from "vitest";

import { parseRuntimeSource } from "../../../services/refact/runtimeCoordinator";
import { server } from "../../../utils/mockServer";
import { render, screen, waitFor } from "../../../utils/test-utils";
import { RuntimeCoordinatorSection } from "./RuntimeCoordinatorSection";

const CONFIG_STATE = {
  config: {
    apiKey: "",
    host: "web" as const,
    lspPort: 8488,
    lspUrl: "http://127.0.0.1:8488",
    surface: "dashboard" as const,
    themeProps: {},
  },
};

const REPOSITORY = { owner: "JegernOUTT", name: "refact", id: 1254216370 };
const DIGEST = "a".repeat(64);
const SHA = "b".repeat(40);
const VERSIONS = {
  runtime: "1.2.3",
  gui: "1.2.3",
  vscode: "1.2.3",
  jetbrains: "1.2.3",
};
const IDENTITY = {
  manifest_digest: DIGEST,
  target: "x86_64-unknown-linux-gnu",
  component_versions: VERSIONS,
};
const UNKNOWN = { state: "unknown" } as const;

function statusResponse(overrides: Record<string, unknown> = {}) {
  return {
    status: {
      selection_revision: 0,
      selected: UNKNOWN,
      installed: UNKNOWN,
      staged: UNKNOWN,
      running: UNKNOWN,
      ...overrides,
    },
    target: "x86_64-unknown-linux-gnu",
    diagnostics: ["runtime selection absent"],
    activation_supported: false,
    restart_dispatched: false,
  };
}

const selectedStable = {
  selection_revision: 3,
  selected: {
    state: "known",
    value: { kind: "channel", repository: REPOSITORY, channel: "stable" },
  },
  running: {
    state: "known",
    value: { pid: 4242, reported_versions: UNKNOWN },
  },
};

const plan = {
  selection_revision: 3,
  identity: IDENTITY,
  source: {
    repository: REPOSITORY,
    workflow: { path: ".github/workflows/agent_engine_release.yml", id: 7 },
    run: { id: 9001, attempt: 2 },
    source_sha: SHA,
    status: "completed",
    conclusion: "success",
  },
  manifest_tests: "not_run",
  qualification: "untested",
  public_qualification_verified: false,
  together: ["runtime_cli", "daemon", "workers", "embedded_gui"],
  ide_owned_not_updated: ["vscode", "jetbrains"],
  activation_supported: false,
  restart_dispatched: false,
};

function renderSection() {
  return render(<RuntimeCoordinatorSection />, {
    preloadedState: CONFIG_STATE,
  });
}

function countForbidden() {
  const forbidden: string[] = [];
  server.use(
    http.post("*/daemon/v1/update/install", ({ request }) => {
      forbidden.push(request.url);
      return HttpResponse.json({}, { status: 500 });
    }),
    http.post("*/daemon/v1/restart", ({ request }) => {
      forbidden.push(request.url);
      return HttpResponse.json({}, { status: 500 });
    }),
    http.post("*/daemon/v1/settings", ({ request }) => {
      forbidden.push(request.url);
      return HttpResponse.json({}, { status: 500 });
    }),
  );
  return forbidden;
}

describe("RuntimeCoordinatorSection", () => {
  let statusUrls: string[];
  let posts: { path: string; body: unknown }[];

  beforeEach(() => {
    statusUrls = [];
    posts = [];
  });

  function useStatus(body: unknown, init?: ResponseInit) {
    server.use(
      http.get("*/daemon/v1/runtime/status", ({ request }) => {
        statusUrls.push(request.url);
        return HttpResponse.json(body as Record<string, unknown>, init);
      }),
    );
  }

  function usePost(path: string, respond: () => Response | Promise<Response>) {
    server.use(
      http.post(`*/daemon/v1/runtime/${path}`, async ({ request }) => {
        posts.push({ path, body: await request.json() });
        return respond();
      }),
    );
  }

  it("renders every observation as Unknown and performs no writes on load", async () => {
    useStatus(statusResponse());
    const forbidden = countForbidden();
    renderSection();

    const list = await screen.findByRole("list", {
      name: "Runtime diagnostics",
    });
    expect(list).toHaveTextContent("runtime selection absent");
    expect(screen.getAllByText("Unknown")).toHaveLength(4);
    expect(
      screen.getByRole("button", { name: "Resolve update plan" }),
    ).toBeDisabled();
    expect(
      screen.getByRole("button", { name: "Stage runtime" }),
    ).toBeDisabled();
    expect(screen.getByText(/Activation is unavailable/)).toBeVisible();
    expect(posts).toEqual([]);
    expect(forbidden).toEqual([]);
    expect(statusUrls).toHaveLength(1);
    expect(new URL(statusUrls[0]).search).toBe("");
  });

  it("shows running PID without inferring component versions", async () => {
    useStatus(statusResponse(selectedStable));
    renderSection();

    expect(
      await screen.findByText("PID 4242 · component versions unknown"),
    ).toBeInTheDocument();
    expect(
      screen.getByText("JegernOUTT/refact · stable channel"),
    ).toBeInTheDocument();
    expect(screen.getAllByText("Unknown")).toHaveLength(2);
  });

  it("selects the fixed repository channel with the current revision", async () => {
    useStatus(statusResponse({ selection_revision: 3 }));
    usePost("select", () =>
      HttpResponse.json({
        schema_version: 1,
        revision: 4,
        selection: {
          kind: "channel",
          repository: REPOSITORY,
          channel: "stable",
        },
      }),
    );
    const { user } = renderSection();

    await user.click(
      await screen.findByRole("button", { name: "Select source" }),
    );

    await waitFor(() => {
      expect(posts).toEqual([
        {
          path: "select",
          body: {
            expected_revision: 3,
            selection: {
              kind: "channel",
              repository: REPOSITORY,
              channel: "stable",
            },
          },
        },
      ]);
    });
    await waitFor(() => expect(statusUrls.length).toBe(2));
  });

  it("rejects invalid sources locally without calling the daemon", async () => {
    useStatus(statusResponse());
    const { user } = renderSection();
    const input = await screen.findByRole("textbox", {
      name: "Runtime source URL",
    });

    await user.clear(input);
    await user.type(input, "https://github.com/other/refact");
    await user.click(screen.getByRole("button", { name: "Select source" }));

    expect(await screen.findByText(/Invalid source/)).toBeInTheDocument();
    expect(posts).toEqual([]);
  });

  it("reports daemon authentication without changing auth settings", async () => {
    useStatus(statusResponse(selectedStable));
    usePost("resolve", () => new HttpResponse(null, { status: 401 }));
    const forbidden = countForbidden();
    const { user } = renderSection();

    await user.click(
      await screen.findByRole("button", { name: "Resolve update plan" }),
    );

    expect(
      await screen.findByText(
        "Runtime mutations require native daemon user authentication; authenticate via the daemon interface or use the refact runtime CLI.",
      ),
    ).toBeInTheDocument();
    expect(posts).toHaveLength(1);
    expect(forbidden).toEqual([]);
  });

  it("shows the backend Actions authentication 409 with refresh guidance", async () => {
    useStatus(
      statusResponse({
        ...selectedStable,
        selected: {
          state: "known",
          value: {
            kind: "pinned",
            repository: REPOSITORY,
            source: { kind: "run", id: 9001, attempt: 1 },
          },
        },
      }),
    );
    usePost("resolve", () =>
      HttpResponse.json(
        { error: "public resolution failed: AuthenticationRequired" },
        { status: 409 },
      ),
    );
    const { user } = renderSection();

    expect(
      await screen.findByText("JegernOUTT/refact · Actions run 9001 attempt 1"),
    ).toBeInTheDocument();
    await user.click(
      screen.getByRole("button", { name: "Resolve update plan" }),
    );

    expect(
      await screen.findByText(
        "public resolution failed: AuthenticationRequired. Runtime status was refreshed; review it before retrying.",
      ),
    ).toBeInTheDocument();
    expect(screen.queryByLabelText("Update plan")).toBeNull();
    await waitFor(() => expect(statusUrls.length).toBe(2));
    expect(posts).toHaveLength(1);
  });

  it("invalidates status on a stale 409 without retrying the mutation", async () => {
    useStatus(statusResponse(selectedStable));
    usePost("stage", () =>
      HttpResponse.json(
        { error: "selection revision conflict" },
        { status: 409 },
      ),
    );
    const { user } = renderSection();

    await user.click(
      await screen.findByRole("button", { name: "Stage runtime" }),
    );

    expect(
      await screen.findByText(
        "selection revision conflict. Runtime status was refreshed; review it before retrying.",
      ),
    ).toBeInTheDocument();
    await waitFor(() => expect(statusUrls.length).toBe(2));
    expect(posts.filter((post) => post.path === "stage")).toHaveLength(1);
  });

  it("clears the stage key and plan after a stale 409", async () => {
    let stagedKnown = false;
    server.use(
      http.get("*/daemon/v1/runtime/status", ({ request }) => {
        statusUrls.push(request.url);
        return HttpResponse.json(
          statusResponse(
            stagedKnown
              ? {
                  ...selectedStable,
                  staged: { state: "known", value: IDENTITY },
                }
              : selectedStable,
          ),
        );
      }),
    );
    let stageCalls = 0;
    usePost("stage", () => {
      stageCalls += 1;
      if (stageCalls > 1) {
        stagedKnown = false;
        return HttpResponse.json(
          { error: "selection revision conflict" },
          { status: 409 },
        );
      }
      stagedKnown = true;
      return HttpResponse.json({
        plan,
        staged: {
          selection_revision: 3,
          manifest_digest: DIGEST,
          target: "x86_64-unknown-linux-gnu",
          runtime_sha256: "c".repeat(64),
          runtime_size: 1024,
        },
      });
    });
    const { user } = renderSection();

    await user.click(
      await screen.findByRole("button", { name: "Stage runtime" }),
    );
    expect(await screen.findByText(/\(cached stage\)$/)).toBeVisible();
    expect(screen.getByLabelText("Update plan")).toBeVisible();

    await user.click(screen.getByRole("button", { name: "Stage runtime" }));

    expect(
      await screen.findByText(
        "selection revision conflict. Runtime status was refreshed; review it before retrying.",
      ),
    ).toBeInTheDocument();
    await waitFor(() =>
      expect(new URL(statusUrls[statusUrls.length - 1]).search).toBe(""),
    );
    expect(screen.queryByLabelText("Update plan")).toBeNull();
    expect(screen.queryByText(/Cached only;/)).toBeNull();
    await waitFor(() => expect(screen.getAllByText("Unknown")).toHaveLength(2));
    expect(stageCalls).toBe(2);
  });

  it("shows the plan, then stages without claiming installation and requeries with the stage key", async () => {
    let stagedKnown = false;
    server.use(
      http.get("*/daemon/v1/runtime/status", ({ request }) => {
        statusUrls.push(request.url);
        return HttpResponse.json(
          statusResponse(
            stagedKnown
              ? {
                  ...selectedStable,
                  staged: { state: "known", value: IDENTITY },
                }
              : selectedStable,
          ),
        );
      }),
    );
    usePost("resolve", () => HttpResponse.json(plan));
    let releaseStage: () => void = () => undefined;
    usePost(
      "stage",
      () =>
        new Promise<Response>((resolve) => {
          releaseStage = () => {
            stagedKnown = true;
            resolve(
              HttpResponse.json({
                plan,
                staged: {
                  selection_revision: 3,
                  manifest_digest: DIGEST,
                  target: "x86_64-unknown-linux-gnu",
                  runtime_sha256: "c".repeat(64),
                  runtime_size: 1024,
                },
              }),
            );
          };
        }),
    );
    const forbidden = countForbidden();
    const { user } = renderSection();

    await user.click(
      await screen.findByRole("button", { name: "Resolve update plan" }),
    );
    expect(await screen.findByText("Untested")).toBeInTheDocument();
    expect(
      screen.getByText("No public qualification evidence"),
    ).toBeInTheDocument();
    expect(screen.getByText("Manifest tests: not run")).toBeInTheDocument();
    expect(screen.getByText(new RegExp(`commit ${SHA}`))).toBeInTheDocument();
    expect(screen.getByText(/run 9001 attempt 2/)).toBeInTheDocument();
    expect(
      screen.getByText("runtime_cli, daemon, workers, embedded_gui"),
    ).toBeInTheDocument();
    expect(screen.getByText("vscode, jetbrains")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Stage runtime" }));
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Resolve update plan" }),
      ).toBeDisabled(),
    );
    expect(
      screen.getByRole("button", { name: "Select source" }),
    ).toBeDisabled();
    releaseStage();

    expect(
      await screen.findByText(
        "Cached x86_64-unknown-linux-gnu runtime (sha256 " +
          "c".repeat(64) +
          ", 1024 bytes). Cached only; this action did not install or restart the runtime.",
      ),
    ).toBeInTheDocument();
    expect(await screen.findByText(/\(cached stage\)$/)).toBeInTheDocument();
    const keyed = new URL(statusUrls[statusUrls.length - 1]);
    expect(keyed.searchParams.get("manifest_digest")).toBe(DIGEST);
    expect(keyed.searchParams.get("selection_revision")).toBe("3");
    expect(screen.queryByText(/^Installed$/)).toBeInTheDocument();
    expect(
      screen.queryByText(/not installed|not running|successfully installed/i),
    ).toBeNull();
    expect(posts.map((post) => post.path)).toEqual(["resolve", "stage"]);
    expect(posts[1].body).toEqual({ expected_revision: 3 });
    expect(forbidden).toEqual([]);
  });
});

describe("parseRuntimeSource", () => {
  it("maps fixed-repository URLs to exact selections", () => {
    expect(
      parseRuntimeSource("https://github.com/JegernOUTT/refact", "nightly"),
    ).toEqual({ kind: "channel", repository: REPOSITORY, channel: "nightly" });
    expect(
      parseRuntimeSource(
        "https://github.com/JegernOUTT/refact/releases/tag/engine/v1.2.3",
        "stable",
      ),
    ).toEqual({
      kind: "pinned",
      repository: REPOSITORY,
      source: { kind: "release", tag: "engine/v1.2.3" },
    });
    expect(
      parseRuntimeSource(
        "https://github.com/JegernOUTT/refact/actions/runs/55/attempts/2",
        "stable",
      ),
    ).toEqual({
      kind: "pinned",
      repository: REPOSITORY,
      source: { kind: "run", id: 55, attempt: 2 },
    });
  });

  it("rejects unsafe or foreign sources", () => {
    for (const input of [
      "http://github.com/JegernOUTT/refact",
      "https://github.com:443/JegernOUTT/refact",
      "https://github.com:/JegernOUTT/refact",
      "https://user@github.com/JegernOUTT/refact",
      "https://%75ser:pw@github.com/JegernOUTT/refact",
      "https://GITHUB.com/JegernOUTT/refact",
      "https://github.com./JegernOUTT/refact",
      "https://github.com//JegernOUTT/refact",
      "https://github.com/JegernOUTT/refact/",
      "https://github.com/%4AegernOUTT/refact/releases/tag/%3A",
      "https://github.com/JegernOUTT/refact?x=1",
      "https://github.com/JegernOUTT/refact#frag",
      "https://github.com/JegernOUTT/refact/releases/tag/..",
      "https://github.com/JegernOUTT/refact/releases/tag/%2E%2E",
      "https://github.com/JegernOUTT/refact/releases/tag/a%2Fb",
      "https://github.com/JegernOUTT/refact/releases/tag/%ZZ",
      "https://github.com/JegernOUTT/refact/actions/runs/0",
      "https://github.com/JegernOUTT/refact/actions/runs/1e3",
      "https://github.com/JegernOUTT/refact/actions/runs/5/attempts",
      "https://github.com/JegernOUTT/refact/actions/runs/99999999999999999999",
      "https://github.com/JegernOUTT/refact/actions/runs/9007199254740993",
      "https://github.com/JegernOUTT/refact/actions/runs/5/attempts/9007199254740993",
      "https://github.com/JegernOUTT/other",
      "/usr/local/bin/refact",
      " https://github.com/JegernOUTT/refact",
    ]) {
      expect(parseRuntimeSource(input, "stable")).toBeNull();
    }
  });
});
