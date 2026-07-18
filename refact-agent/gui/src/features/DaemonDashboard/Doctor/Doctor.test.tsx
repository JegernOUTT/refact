import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";

import { setUpStore } from "../../../app/store";
import type { DaemonWorker } from "../../../services/refact/daemon";
import type {
  ProviderDefaults,
  ProviderListItem,
} from "../../../services/refact/providers";
import { server } from "../../../utils/mockServer";
import { render, screen, waitFor } from "../../../utils/test-utils";
import {
  checkFailedFinding,
  collectProviderModels,
  providerHealthFinding,
  quotaFinding,
  staleDefaultModelFindings,
} from "./clientChecks";
import { DoctorPage } from "./DoctorPage";
import {
  buildDefaultsUpdate,
  PRUNE_CACHES_COMMAND,
  resolveServerFixAction,
} from "./fixActions";

const config = {
  apiKey: "",
  host: "web" as const,
  lspPort: 8488,
  lspUrl: "https://daemon.example.test",
  surface: "dashboard" as const,
  themeProps: {},
};

const project = { projectId: "refact", projectSlug: "refact" };

function worker(projectId: string, state: string): DaemonWorker {
  return {
    project_id: projectId,
    slug: projectId,
    root: `/work/${projectId}`,
    pinned: false,
    last_active_ms: 1,
    state,
    pid: state === "ready" ? 10 : null,
    http_port: state === "ready" ? 8001 : null,
    lsp_port: state === "ready" ? 9001 : null,
    lsp_clients: 0,
    busy_chats: 0,
    exec_running: 0,
    live_proxy_streams: 0,
    cron_next_fire_ms: null,
    idle_deadline_ms: null,
    last_status_report_ms: 1,
    last_error: null,
  };
}

function providerItem(name: string, baseProvider = name): ProviderListItem {
  return {
    name,
    base_provider: baseProvider,
    display_name: name,
    enabled: true,
    readonly: false,
    has_credentials: true,
    status: "configured",
    model_count: 1,
  };
}

function makeDefaults(chatModel?: string): ProviderDefaults {
  return {
    chat: chatModel === undefined ? {} : { model: chatModel },
    chat_model_2: {},
    task_planner_agent_model: {},
    chat_light: {},
    chat_thinking: {},
  };
}

function modelsResponse(names: string[]) {
  return {
    chat_models: names.map((name) => ({
      name,
      enabled: true,
      removable: false,
      user_configured: false,
    })),
    completion_models: [],
    embedding_model: null,
  };
}

function doctorHandler(findings: unknown[] = []) {
  return http.get("https://daemon.example.test/daemon/v1/doctor", () =>
    HttpResponse.json({ findings, generated_at_ms: 1 }),
  );
}

function workersHandler(workers: DaemonWorker[]) {
  return http.get("https://daemon.example.test/daemon/v1/workers", () =>
    HttpResponse.json(workers),
  );
}

function renderDoctor() {
  return render(<DoctorPage />, { store: setUpStore({ config }) });
}

describe("Doctor client checks (pure)", () => {
  it("flags default models missing from the available set as critical", () => {
    const defaults = makeDefaults("refact/gone-model");
    const findings = staleDefaultModelFindings(project, defaults, [
      "openai/gpt-4o",
    ]);
    expect(findings).toHaveLength(1);
    expect(findings[0].severity).toBe("critical");
    expect(findings[0].message).toBe(
      "Default model 'refact/gone-model' not found on refact",
    );
    expect(findings[0].fix).toMatchObject({
      kind: "stale_default_model",
      slotKey: "chat",
      staleModel: "refact/gone-model",
      availableModels: ["openai/gpt-4o"],
    });
  });

  it("does not flag present models or judge with an empty model set", () => {
    expect(
      staleDefaultModelFindings(project, makeDefaults("openai/gpt-4o"), [
        "openai/gpt-4o",
      ]),
    ).toHaveLength(0);
    expect(
      staleDefaultModelFindings(project, makeDefaults("refact/gone-model"), []),
    ).toHaveLength(0);
    expect(
      staleDefaultModelFindings(project, makeDefaults(), ["m"]),
    ).toHaveLength(0);
  });

  it("collects qualified model ids from a models response", () => {
    expect(
      collectProviderModels("openai", modelsResponse(["gpt-4o", "gpt-4.1"])),
    ).toEqual(["openai/gpt-4o", "openai/gpt-4.1"]);
    expect(collectProviderModels("openai", null)).toEqual([]);
  });

  it("maps provider health results to warnings only when unhealthy", () => {
    expect(
      providerHealthFinding(project, "openrouter", { status: 401, body: null }),
    ).toMatchObject({
      severity: "warning",
      fix: { kind: "open_project_providers", projectId: "refact" },
    });
    expect(
      providerHealthFinding(project, "openrouter", {
        status: 200,
        body: { ok: false, message: "invalid key" },
      }),
    ).toMatchObject({ severity: "warning", detail: "invalid key" });
    expect(
      providerHealthFinding(project, "openrouter", {
        status: 200,
        body: { ok: true },
      }),
    ).toBeNull();
    expect(
      providerHealthFinding(project, "openrouter", { status: 400, body: null }),
    ).toBeNull();
  });

  it("flags exhausted token plans as warnings", () => {
    expect(
      quotaFinding(project, "openrouter", { data: { remaining: 0 } }),
    ).toMatchObject({ severity: "warning" });
    expect(
      quotaFinding(project, "openrouter", { data: { limit: 10, usage: 12 } }),
    ).toMatchObject({ severity: "warning" });
    expect(
      quotaFinding(project, "openrouter", { data: { remaining: 5 } }),
    ).toBeNull();
    expect(quotaFinding(project, "openrouter", { data: {} })).toBeNull();
  });

  it("degrades failed checks to info findings", () => {
    expect(checkFailedFinding(project, "provider list")).toMatchObject({
      severity: "info",
      message: "Check failed: provider list on refact",
    });
  });
});

describe("Doctor fix actions", () => {
  it("resolves server fix action ids", () => {
    expect(resolveServerFixAction("restart_worker:refact")).toEqual({
      kind: "restart_worker",
      projectId: "refact",
    });
    expect(resolveServerFixAction("run_update")).toEqual({
      kind: "run_update",
    });
    expect(resolveServerFixAction("open_settings")).toEqual({
      kind: "open_settings",
    });
    expect(resolveServerFixAction("prune_caches")).toEqual({
      kind: "copy_command",
      command: PRUNE_CACHES_COMMAND,
    });
    expect(resolveServerFixAction("something_else")).toBeNull();
    expect(resolveServerFixAction(null)).toBeNull();
  });

  it("patches only the stale slot when building the defaults update", () => {
    const defaults: ProviderDefaults = {
      ...makeDefaults("refact/gone-model"),
      chat_light: { model: "openai/gpt-4o-mini", temperature: 0.2 },
    };
    const updated = buildDefaultsUpdate(
      {
        kind: "stale_default_model",
        projectId: "refact",
        projectSlug: "refact",
        slotKey: "chat",
        staleModel: "refact/gone-model",
        availableModels: ["openai/gpt-4o"],
        defaults,
      },
      "openai/gpt-4o",
    );
    expect(updated.chat).toEqual({ model: "openai/gpt-4o" });
    expect(updated.chat_light).toEqual({
      model: "openai/gpt-4o-mini",
      temperature: 0.2,
    });
  });
});

describe("Doctor page", () => {
  it("detects and fixes a stale default model end-to-end", async () => {
    let chatModel: string | undefined = "refact/gone-model";
    let postedBody: unknown = null;
    server.use(
      workersHandler([worker("refact", "ready")]),
      doctorHandler(),
      http.get("https://daemon.example.test/p/refact/v1/providers", () =>
        HttpResponse.json({ providers: [providerItem("openai")] }),
      ),
      http.get("https://daemon.example.test/p/refact/v1/defaults", () =>
        HttpResponse.json(makeDefaults(chatModel)),
      ),
      http.get("https://daemon.example.test/p/refact/v1/models", () =>
        HttpResponse.json(modelsResponse(["gpt-4o"])),
      ),
      http.post(
        "https://daemon.example.test/p/refact/v1/defaults",
        async ({ request }) => {
          postedBody = await request.json();
          chatModel = (postedBody as ProviderDefaults).chat.model;
          return HttpResponse.json({ success: true });
        },
      ),
    );

    const view = renderDoctor();

    expect(
      await screen.findByText(
        "Default model 'refact/gone-model' not found on refact",
      ),
    ).toBeInTheDocument();

    await view.user.click(screen.getByRole("button", { name: "Apply" }));

    await waitFor(() => expect(postedBody).not.toBeNull());
    expect(postedBody).toEqual({
      ...makeDefaults(),
      chat: { model: "openai/gpt-4o" },
    });
    expect(await screen.findByText("All checks passed 🩺")).toBeInTheDocument();
  });

  it("renders server findings and routes fix actions", async () => {
    let restartCalled = false;
    server.use(
      workersHandler([worker("refact", "ready")]),
      doctorHandler([
        {
          id: "worker_crash",
          severity: "warning",
          message: "Worker refact crashed",
          detail: null,
          fix_action: "restart_worker:refact",
        },
        {
          id: "lan_without_auth",
          severity: "critical",
          message: "LAN enabled without auth",
          detail: null,
          fix_action: "open_settings",
        },
      ]),
      http.get("https://daemon.example.test/p/refact/v1/providers", () =>
        HttpResponse.json({ providers: [providerItem("openai")] }),
      ),
      http.get("https://daemon.example.test/p/refact/v1/defaults", () =>
        HttpResponse.json(makeDefaults("openai/gpt-4o")),
      ),
      http.get("https://daemon.example.test/p/refact/v1/models", () =>
        HttpResponse.json(modelsResponse(["gpt-4o"])),
      ),
      http.post(
        "https://daemon.example.test/daemon/v1/projects/refact/restart",
        () => {
          restartCalled = true;
          return HttpResponse.json({
            project_id: "refact",
            pid: 11,
            http_port: 8001,
            lsp_port: 9001,
            state: "ready",
          });
        },
      ),
    );

    const view = renderDoctor();

    expect(
      await screen.findByText("Worker refact crashed"),
    ).toBeInTheDocument();
    expect(screen.getByText("LAN enabled without auth")).toBeInTheDocument();

    await view.user.click(
      screen.getByRole("button", { name: "Restart worker" }),
    );
    await waitFor(() => expect(restartCalled).toBe(true));

    await view.user.click(
      screen.getByRole("button", { name: "Open settings" }),
    );
    expect(view.store.getState().daemonDashboard.navigation.page).toBe(
      "settings",
    );
  });

  it("shows the all-green state when every check passes", async () => {
    server.use(
      workersHandler([worker("refact", "ready")]),
      doctorHandler(),
      http.get("https://daemon.example.test/p/refact/v1/providers", () =>
        HttpResponse.json({ providers: [providerItem("openai")] }),
      ),
      http.get("https://daemon.example.test/p/refact/v1/defaults", () =>
        HttpResponse.json(makeDefaults("openai/gpt-4o")),
      ),
      http.get("https://daemon.example.test/p/refact/v1/models", () =>
        HttpResponse.json(modelsResponse(["gpt-4o"])),
      ),
    );

    renderDoctor();

    expect(await screen.findByText("All checks passed 🩺")).toBeInTheDocument();
  });

  it("degrades a failing client check to an info finding", async () => {
    server.use(
      workersHandler([worker("refact", "ready")]),
      doctorHandler(),
      http.get(
        "https://daemon.example.test/p/refact/v1/providers",
        () => new HttpResponse(null, { status: 500 }),
      ),
    );

    renderDoctor();

    expect(
      await screen.findByText("Check failed: provider list on refact"),
    ).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Info" })).toBeInTheDocument();
  });
});
