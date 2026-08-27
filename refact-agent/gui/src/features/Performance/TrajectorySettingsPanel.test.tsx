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
});
