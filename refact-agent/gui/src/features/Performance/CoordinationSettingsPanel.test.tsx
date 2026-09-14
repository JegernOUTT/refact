import { describe, expect, it } from "vitest";
import { http, HttpResponse } from "msw";
import type { CoordinationSettingsResponse } from "../../services/refact/performance";
import { server } from "../../utils/mockServer";
import { render, screen, waitFor } from "../../utils/test-utils";
import { CoordinationSettingsPanel } from "./CoordinationSettingsPanel";

HTMLElement.prototype.hasPointerCapture = () => false;
HTMLElement.prototype.setPointerCapture = () => undefined;
HTMLElement.prototype.releasePointerCapture = () => undefined;

const preloadedState = {
  config: {
    apiKey: null,
    host: "web" as const,
    lspPort: 8001,
    themeProps: { appearance: "dark" as const },
  },
};
const global = {
  max_active_engine: 32,
  root_fairness_quantum: 1,
  max_resume_burst: 4,
  root_weights: {},
  max_root_weight_entries: 4096,
  launch_context_tokens: 24_000,
  launch_context_messages: 1024,
  launch_context_entries: 256,
  launch_context_bytes: 16 * 1024 * 1024,
  launch_source_bytes: 4 * 1024 * 1024,
  auto_inject_enabled: true,
  future_setting: 99,
};
function response(
  scope: "global" | "project" = "global",
): CoordinationSettingsResponse {
  return {
    scope,
    path: "/settings",
    config: scope === "global" ? global : { max_active_engine: 48 },
    global,
    effective:
      scope === "global" ? global : { ...global, max_active_engine: 48 },
    defaults: global,
    fields: [
      {
        name: "root_weights",
        value_type: "integer_map",
        minimum: 1,
        maximum: 1024,
        apply_mode: "live",
      },
      {
        name: "max_active_engine",
        value_type: "integer",
        minimum: 1,
        maximum: 4096,
        apply_mode: "live",
      },
      ...["root_fairness_quantum", "max_resume_burst"].map((name) => ({
        name,
        value_type: "integer",
        minimum: 1,
        maximum: 1024,
        apply_mode: "live",
      })),
      ...[
        ["launch_context_tokens", 1_000_000],
        ["launch_context_messages", 100_000],
        ["launch_context_entries", 100_000],
        ["launch_context_bytes", 1_073_741_824],
        ["launch_source_bytes", 1_073_741_824],
      ].map(([name, maximum]) => ({
        name: String(name),
        value_type: "integer",
        minimum: 1,
        maximum: Number(maximum),
        apply_mode: "live",
      })),
      {
        name: "auto_inject_enabled",
        value_type: "boolean",
        apply_mode: "live",
      },
    ],
    project_available: true,
    inherited: scope === "project" ? ["auto_inject_enabled"] : [],
  };
}

describe("CoordinationSettingsPanel", () => {
  it("exposes and saves each launch context bound without dropping other settings", async () => {
    let saved: unknown;
    server.use(
      http.get("*/v1/coordination-settings", () =>
        HttpResponse.json(response()),
      ),
      http.post("*/v1/coordination-settings", async ({ request }) => {
        saved = await request.json();
        return HttpResponse.json(response());
      }),
    );
    const { user } = render(<CoordinationSettingsPanel />, { preloadedState });
    const tokenInput = await screen.findByRole("spinbutton", {
      name: "Launch context tokens",
    });
    expect(tokenInput).toHaveValue(24_000);
    for (const [name, value] of [
      ["Launch context messages", 1024],
      ["Launch context entries", 256],
      ["Launch context bytes", 16 * 1024 * 1024],
      ["Launch source bytes", 4 * 1024 * 1024],
    ] as const) {
      expect(screen.getByRole("spinbutton", { name })).toHaveValue(value);
    }
    await user.clear(tokenInput);
    await user.type(tokenInput, "1000001");
    expect(
      screen.getByRole("button", { name: "Save coordination settings" }),
    ).toBeDisabled();
    expect(saved).toBeUndefined();
    await user.clear(tokenInput);
    await user.type(tokenInput, "48000");
    await user.click(
      screen.getByRole("button", { name: "Save coordination settings" }),
    );
    await waitFor(() =>
      expect(saved).toEqual({
        config: { ...global, launch_context_tokens: 48_000 },
      }),
    );
  });

  it("edits named root weights with bounded validated controls", async () => {
    let saved: unknown;
    server.use(
      http.get("*/v1/coordination-settings", () =>
        HttpResponse.json(response()),
      ),
      http.post("*/v1/coordination-settings", async ({ request }) => {
        saved = await request.json();
        return HttpResponse.json(response());
      }),
    );
    const { user } = render(<CoordinationSettingsPanel />, { preloadedState });
    await user.type(
      await screen.findByRole("textbox", {
        name: "Root chat ID for scheduling weight",
      }),
      "root-one",
    );
    await user.click(screen.getByRole("button", { name: "Add root weight" }));
    const weight = screen.getByRole("spinbutton", {
      name: "Scheduling weight for root-one",
    });
    await user.clear(weight);
    await user.type(weight, "0");
    expect(
      screen.getByRole("button", { name: "Save coordination settings" }),
    ).toBeDisabled();
    await user.clear(weight);
    await user.type(weight, "3");
    await user.click(
      screen.getByRole("button", { name: "Save coordination settings" }),
    );
    await waitFor(() =>
      expect(saved).toEqual({
        config: { ...global, root_weights: { "root-one": 3 } },
      }),
    );
  });

  it("exposes live fairness controls without replacing effort budgets", async () => {
    let saved: unknown;
    server.use(
      http.get("*/v1/coordination-settings", () =>
        HttpResponse.json(response()),
      ),
      http.post("*/v1/coordination-settings", async ({ request }) => {
        saved = await request.json();
        return HttpResponse.json(response());
      }),
    );
    const { user } = render(<CoordinationSettingsPanel />, { preloadedState });
    const quantum = await screen.findByRole("spinbutton", {
      name: "Root fairness quantum",
    });
    const burst = screen.getByRole("spinbutton", { name: "Max resume burst" });
    expect(quantum).toHaveValue(1);
    expect(burst).toHaveValue(4);
    expect(
      screen.getByText(/interactive top-level chats are not charged/),
    ).toBeVisible();
    expect(
      screen.getByText(/Goal effort budgets remain unlimited/),
    ).toBeVisible();
    await user.clear(quantum);
    await user.type(quantum, "2");
    await user.clear(burst);
    await user.type(burst, "8");
    await user.click(
      screen.getByRole("button", { name: "Save coordination settings" }),
    );
    await waitFor(() =>
      expect(saved).toEqual({
        config: { ...global, root_fairness_quantum: 2, max_resume_burst: 8 },
      }),
    );
  });

  it("saves a changed cap while preserving unrendered fields", async () => {
    let saved: unknown;
    server.use(
      http.get("*/v1/coordination-settings", () =>
        HttpResponse.json(response()),
      ),
      http.post("*/v1/coordination-settings", async ({ request }) => {
        saved = await request.json();
        return HttpResponse.json(response());
      }),
    );
    const { user } = render(<CoordinationSettingsPanel />, { preloadedState });
    const input = await screen.findByRole("spinbutton", {
      name: "Max active engine",
    });
    expect(input).toHaveValue(32);
    await user.clear(input);
    await user.type(input, "64");
    await user.click(
      screen.getByRole("button", { name: "Save coordination settings" }),
    );
    await waitFor(() =>
      expect(saved).toEqual({ config: { ...global, max_active_engine: 64 } }),
    );
    expect(await screen.findByRole("status")).toHaveTextContent(
      "Coordination settings saved",
    );
  });

  it("rejects invalid cap input without making a request", async () => {
    let calls = 0;
    server.use(
      http.get("*/v1/coordination-settings", () =>
        HttpResponse.json(response()),
      ),
      http.post("*/v1/coordination-settings", () => {
        calls++;
        return HttpResponse.json(response());
      }),
    );
    const { user } = render(<CoordinationSettingsPanel />, { preloadedState });
    const input = await screen.findByRole("spinbutton", {
      name: "Max active engine",
    });
    await user.clear(input);
    await user.type(input, "0");
    expect(
      screen.getByRole("button", { name: "Save coordination settings" }),
    ).toBeDisabled();
    expect(calls).toBe(0);
  });

  it("resets project overrides to a sparse inherited configuration", async () => {
    let saved: unknown;
    server.use(
      http.get("*/v1/coordination-settings", ({ request }) =>
        HttpResponse.json(
          response(
            new URL(request.url).searchParams.get("scope") === "project"
              ? "project"
              : "global",
          ),
        ),
      ),
      http.post("*/v1/coordination-settings", async ({ request }) => {
        saved = await request.json();
        return HttpResponse.json({ ...response("project"), config: {} });
      }),
    );
    const { user } = render(<CoordinationSettingsPanel />, { preloadedState });
    await screen.findByRole("spinbutton", { name: "Max active engine" });
    await user.click(
      screen.getByRole("combobox", { name: "Coordination settings scope" }),
    );
    await user.click(
      await screen.findByRole("option", { name: "This project" }),
    );
    await waitFor(() =>
      expect(
        screen.getByRole("spinbutton", { name: "Max active engine" }),
      ).toHaveValue(48),
    );
    await user.click(
      screen.getByRole("button", { name: "Inherit all global settings" }),
    );
    await user.click(
      screen.getByRole("button", { name: "Save coordination settings" }),
    );
    await waitFor(() => expect(saved).toEqual({ config: {} }));
  });
});
