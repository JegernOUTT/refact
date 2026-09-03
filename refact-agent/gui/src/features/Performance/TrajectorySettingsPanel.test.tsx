import { describe, expect, it } from "vitest";
import { http, HttpResponse } from "msw";

import type { TrajectorySettingsResponse } from "../../services/refact/performance";
import { server } from "../../utils/mockServer";
import { render, screen, waitFor } from "../../utils/test-utils";
import { TrajectorySettingsPanel } from "./TrajectorySettingsPanel";

const configState = {
  config: {
    apiKey: null,
    host: "web" as const,
    lspPort: 8001,
    themeProps: { appearance: "dark" as const },
  },
};

function settingsResponse(
  overrides: Partial<TrajectorySettingsResponse> = {},
): TrajectorySettingsResponse {
  const config = {
    internal_traces_keep_per_folder: 200,
    session_idle_timeout_secs: 1800,
    event_channel_capacity: 4096,
    max_parallel_tools: null,
    auto_enrichment_total_token_cap: 1600,
    trajectory_writer_enabled: true,
  };
  return {
    path: "/config/trajectory-settings.yaml",
    config,
    current: config,
    defaults: { ...config, internal_traces_keep_per_folder: 100 },
    environment_precedence: "Environment values take precedence.",
    fields: [
      {
        name: "internal_traces_keep_per_folder",
        value_type: "integer",
        minimum: 10,
        maximum: 10000,
        apply_mode: "live",
      },
      {
        name: "session_idle_timeout_secs",
        value_type: "integer",
        minimum: 60,
        maximum: 86400,
        apply_mode: "live",
      },
      {
        name: "event_channel_capacity",
        value_type: "integer",
        minimum: 16,
        maximum: 1000000,
        apply_mode: "restart_required",
      },
      {
        name: "max_parallel_tools",
        value_type: "integer",
        minimum: 1,
        maximum: 10000,
        apply_mode: "live",
      },
      {
        name: "auto_enrichment_total_token_cap",
        value_type: "integer",
        minimum: 64,
        maximum: 32000,
        apply_mode: "live",
      },
      {
        name: "trajectory_writer_enabled",
        value_type: "boolean",
        apply_mode: "restart_required",
      },
    ],
    ...overrides,
  };
}

const NEW_LIMIT_FIELDS: {
  name: string;
  group: string;
  value: number;
  label: string;
}[] = [
  {
    name: "pp_max_tool_budget_tokens",
    group: "Tool output budgets",
    value: 6000,
    label: "Pp max tool budget tokens",
  },
  {
    name: "pp_max_per_file_budget_tokens",
    group: "Tool output budgets",
    value: 2000,
    label: "Pp max per file budget tokens",
  },
  {
    name: "pp_max_line_length_chars",
    group: "Tool output budgets",
    value: 800,
    label: "Pp max line length chars",
  },
  {
    name: "pp_tokens_for_text_percent",
    group: "Tool output budgets",
    value: 70,
    label: "Pp tokens for text percent",
  },
  {
    name: "cat_max_input_paths",
    group: "File and log reading",
    value: 32,
    label: "Cat max input paths",
  },
  {
    name: "cat_max_lines",
    group: "File and log reading",
    value: 5000,
    label: "Cat max lines",
  },
  {
    name: "cat_max_file_bytes",
    group: "File and log reading",
    value: 1000000,
    label: "Cat max file bytes",
  },
  {
    name: "cat_max_expanded_files",
    group: "File and log reading",
    value: 12,
    label: "Cat max expanded files",
  },
  {
    name: "get_logs_max_tail_bytes",
    group: "File and log reading",
    value: 65536,
    label: "Get logs max tail bytes",
  },
  {
    name: "agent_diff_max_output_bytes",
    group: "File and log reading",
    value: 131072,
    label: "Agent diff max output bytes",
  },
  {
    name: "process_subscribe_preview_bytes",
    group: "File and log reading",
    value: 4096,
    label: "Process subscribe preview bytes",
  },
  {
    name: "hist_search_preview_chars",
    group: "Search and history",
    value: 900,
    label: "Hist search preview chars",
  },
  {
    name: "vecdb_trajectory_split_bytes",
    group: "Search and history",
    value: 32768,
    label: "Vecdb trajectory split bytes",
  },
  {
    name: "planner_qna_question_limit",
    group: "Search and history",
    value: 24,
    label: "Planner qna question limit",
  },
  {
    name: "planner_qna_answer_limit",
    group: "Search and history",
    value: 48,
    label: "Planner qna answer limit",
  },
  {
    name: "git_intel_max_commits",
    group: "Git intelligence",
    value: 2000,
    label: "Git intel max commits",
  },
  {
    name: "git_intel_deep_walk_limit",
    group: "Git intelligence",
    value: 500,
    label: "Git intel deep walk limit",
  },
  {
    name: "git_intel_max_files_per_commit_cochange",
    group: "Git intelligence",
    value: 60,
    label: "Git intel max files per commit cochange",
  },
  {
    name: "git_intel_max_files_per_commit_entropy",
    group: "Git intelligence",
    value: 80,
    label: "Git intel max files per commit entropy",
  },
  {
    name: "codegraph_dead_code_max_results",
    group: "Code graph",
    value: 300,
    label: "Codegraph dead code max results",
  },
  {
    name: "codegraph_exec_flow_max_nodes",
    group: "Code graph",
    value: 900,
    label: "Codegraph exec flow max nodes",
  },
  {
    name: "review_diff_char_cap",
    group: "Code review",
    value: 120000,
    label: "Review diff char cap",
  },
  {
    name: "review_max_diff_patch_bytes",
    group: "Code review",
    value: 262144,
    label: "Review max diff patch bytes",
  },
];

function settingsResponseWithNewLimits(): TrajectorySettingsResponse {
  const base = settingsResponse();
  const extra = Object.fromEntries(
    NEW_LIMIT_FIELDS.map((field) => [field.name, field.value]),
  );
  const config = { ...base.config, ...extra };
  return {
    ...base,
    config,
    current: config,
    defaults: { ...base.defaults, ...extra },
    fields: [
      ...base.fields,
      ...NEW_LIMIT_FIELDS.map((field) => ({
        name: field.name,
        value_type: "integer" as const,
        minimum: 1,
        maximum: 100000000,
        apply_mode: "live" as const,
      })),
    ],
  };
}

function renderPanel() {
  return render(<TrajectorySettingsPanel />, { preloadedState: configState });
}

describe("TrajectorySettingsPanel", () => {
  it("renders API-provided groups, values, and restart labels", async () => {
    server.use(
      http.get("*/v1/trajectory-settings", () =>
        HttpResponse.json(settingsResponse()),
      ),
    );

    renderPanel();

    expect(await screen.findByText("Retention")).toBeInTheDocument();
    expect(screen.getByText("Session lifecycle")).toBeInTheDocument();
    expect(screen.getByText("Chat limits")).toBeInTheDocument();
    expect(screen.getByText("Enrichment caps")).toBeInTheDocument();
    expect(screen.getByText("Performance optimizations")).toBeInTheDocument();
    expect(screen.getByDisplayValue("200")).toBeInTheDocument();
    expect(screen.getAllByText("Requires restart").length).toBeGreaterThan(0);
    expect(screen.getByText(/enabled by default/i)).toBeInTheDocument();
  });

  it("blocks save for invalid values and shows the advertised range", async () => {
    let saves = 0;
    server.use(
      http.get("*/v1/trajectory-settings", () =>
        HttpResponse.json(settingsResponse()),
      ),
      http.post("*/v1/trajectory-settings", () => {
        saves += 1;
        return HttpResponse.json(settingsResponse());
      }),
    );
    const { user } = renderPanel();

    const input = await screen.findByRole("spinbutton", {
      name: "Internal traces keep per folder",
    });
    await user.clear(input);
    await user.type(input, "9");

    expect(screen.getByText("Allowed range: 10–10,000.")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Save settings" }),
    ).toBeDisabled();
    expect(saves).toBe(0);
  });

  it("saves the edited complete config and shows backend field failures", async () => {
    let savedBody: unknown = null;
    server.use(
      http.get("*/v1/trajectory-settings", () =>
        HttpResponse.json(settingsResponse()),
      ),
      http.post("*/v1/trajectory-settings", async ({ request }) => {
        savedBody = await request.json();
        return HttpResponse.json(
          { detail: "session_idle_timeout_secs is rejected by this engine" },
          { status: 400 },
        );
      }),
    );
    const { user } = renderPanel();

    const input = await screen.findByRole("spinbutton", {
      name: "Session idle timeout secs",
    });
    await user.clear(input);
    await user.type(input, "2400");
    await user.click(screen.getByRole("button", { name: "Save settings" }));

    await waitFor(() =>
      expect(savedBody).toEqual({
        internal_traces_keep_per_folder: 200,
        session_idle_timeout_secs: 2400,
        event_channel_capacity: 4096,
        max_parallel_tools: null,
        auto_enrichment_total_token_cap: 1600,
        trajectory_writer_enabled: true,
      }),
    );
    expect(
      await screen.findByText(
        "session_idle_timeout_secs is rejected by this engine",
      ),
    ).toBeInTheDocument();
  });

  it("preserves an unrendered server setting when saving an edited field", async () => {
    let savedBody: unknown = null;
    const response = settingsResponse();
    const config = { ...response.config, some_future_engine_setting: 42 };
    server.use(
      http.get("*/v1/trajectory-settings", () =>
        HttpResponse.json({
          ...response,
          config,
          current: config,
          defaults: { ...response.defaults, some_future_engine_setting: 42 },
        }),
      ),
      http.post("*/v1/trajectory-settings", async ({ request }) => {
        savedBody = await request.json();
        return HttpResponse.json(response);
      }),
    );
    const { user } = renderPanel();

    const input = await screen.findByRole("spinbutton", {
      name: "Session idle timeout secs",
    });
    await user.clear(input);
    await user.type(input, "2400");
    await user.click(screen.getByRole("button", { name: "Save settings" }));

    await waitFor(() =>
      expect(savedBody).toEqual({
        ...config,
        session_idle_timeout_secs: 2400,
      }),
    );
  });

  it("resets the draft to API-provided defaults", async () => {
    server.use(
      http.get("*/v1/trajectory-settings", () =>
        HttpResponse.json(settingsResponse()),
      ),
    );
    const { user } = renderPanel();

    const input = await screen.findByRole("spinbutton", {
      name: "Internal traces keep per folder",
    });
    await user.clear(input);
    await user.type(input, "250");
    await user.click(screen.getByRole("button", { name: "Reset to defaults" }));

    expect(input).toHaveValue(100);
  });

  it("renders the new limit groups with their fields", async () => {
    server.use(
      http.get("*/v1/trajectory-settings", () =>
        HttpResponse.json(settingsResponseWithNewLimits()),
      ),
    );

    renderPanel();

    expect(await screen.findByText("Tool output budgets")).toBeInTheDocument();
    for (const groupTitle of [
      "File and log reading",
      "Search and history",
      "Git intelligence",
      "Code graph",
      "Code review",
    ]) {
      expect(screen.getByText(groupTitle)).toBeInTheDocument();
    }

    for (const field of NEW_LIMIT_FIELDS) {
      const input = screen.getByRole("spinbutton", { name: field.label });
      expect(input).toHaveValue(field.value);
      const section = input.closest("section");
      expect(section, `no section for "${field.name}"`).not.toBeNull();
      expect(section).toHaveTextContent(field.group);
    }
  });

  it("still routes the optimization warning to the Performance optimizations group", async () => {
    server.use(
      http.get("*/v1/trajectory-settings", () =>
        HttpResponse.json(settingsResponseWithNewLimits()),
      ),
    );
    const { user } = renderPanel();

    const toggle = await screen.findByRole("switch", {
      name: "Trajectory writer enabled",
    });
    await user.click(toggle);

    const warnings = screen.getAllByText(
      /Disabled optimization reduces performance\./,
    );
    expect(warnings).toHaveLength(1);
    const section = warnings[0].closest("section");
    expect(section).not.toBeNull();
    expect(section).toHaveTextContent("Performance optimizations");
  });

  it("still renders unknown server fields under Additional settings", async () => {
    const response = settingsResponseWithNewLimits();
    const config = { ...response.config, some_future_engine_setting: 42 };
    server.use(
      http.get("*/v1/trajectory-settings", () =>
        HttpResponse.json({
          ...response,
          config,
          current: config,
          defaults: { ...response.defaults, some_future_engine_setting: 42 },
          fields: [
            ...response.fields,
            {
              name: "some_future_engine_setting",
              value_type: "integer",
              minimum: 1,
              maximum: 1000,
              apply_mode: "live",
            },
          ],
        }),
      ),
    );

    renderPanel();

    expect(await screen.findByText("Additional settings")).toBeInTheDocument();
    const input = screen.getByRole("spinbutton", {
      name: "Some future engine setting",
    });
    expect(input.closest("section")).toHaveTextContent("Additional settings");
  });
});
