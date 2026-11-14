import { HttpResponse, http } from "msw";
import { afterEach, describe, expect, it, vi } from "vitest";

import type {
  PrivacyPolicy,
  PrivacyPolicyResponse,
  PrivacyStatusResponse,
} from "../../services/refact/privacy";
import { integrationsApi } from "../../services/refact/integrations";
import { mcpMarketplaceApi } from "../../services/refact/mcpMarketplace";
import { server } from "../../utils/mockServer";
import { render, screen, waitFor, within } from "../../utils/test-utils";
import { PrivacySettingsSection } from "./PrivacySettingsSection";

const policy: PrivacyPolicy = {
  blocked: ["*.blocked"],
  zones: [
    {
      name: "secrets",
      patterns: [".env*"],
      send_to: ["trusted"],
      on_shell_read: "withhold",
    },
    {
      name: "normal",
      patterns: ["*"],
      send_to: ["*"],
      on_shell_read: "ask",
    },
  ],
  subagents: { report_declassifies: true },
  tool_access: { providers: {} },
};

const response: PrivacyPolicyResponse = {
  policy,
  destinations: [
    {
      id: "trusted",
      kind: "provider",
      display_name: "trusted",
    },
    {
      id: "build-mcp",
      kind: "mcp",
      display_name: "build-mcp",
    },
  ],
  match_counts: { secrets: 2, normal: 7 },
  error: null,
  source_paths: ["/home/user/.config/refact/privacy.yaml"],
  has_project_overrides: false,
};

const status: PrivacyStatusResponse = {
  platform: "linux",
  observation: {
    platform_supported: true,
    runtime_available: true,
    last_error: null,
  },
  config_error: null,
};

function mockPolicyEndpoints(
  onSave: (policy: PrivacyPolicy) => void,
  options: { failSave?: boolean } = {},
) {
  let currentPolicy = policy;
  server.use(
    http.get("*/v1/privacy/policy", () =>
      HttpResponse.json({ ...response, policy: currentPolicy }),
    ),
    http.get("*/v1/privacy/status", () => HttpResponse.json(status)),
    http.post("*/v1/privacy/policy", async ({ request }) => {
      const saved = (await request.json()) as PrivacyPolicy;
      onSave(saved);
      if (options.failSave) {
        return HttpResponse.json(
          { detail: "privacy.yaml is read-only" },
          { status: 500 },
        );
      }
      currentPolicy = saved;
      return HttpResponse.json({
        ...response,
        policy: saved,
      } satisfies PrivacyPolicyResponse);
    }),
  );
}

const realSetTimeout = setTimeout;

async function advanceUntil(done: () => boolean, maxSteps = 80) {
  for (let step = 0; step < maxSteps && !done(); step += 1) {
    await vi.advanceTimersByTimeAsync(500);
    await new Promise<void>((resolve) => realSetTimeout(resolve, 5));
  }
}

describe("PrivacySettingsSection", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("groups destinations by kind and saves a zone toggle only after Save changes", async () => {
    let savedPolicy: PrivacyPolicy | null = null;
    mockPolicyEndpoints((saved) => {
      savedPolicy = saved;
    });

    const view = render(<PrivacySettingsSection />);

    const providerTab = await screen.findByRole("button", {
      name: /Model providers/,
    });
    expect(providerTab).toHaveAttribute("aria-pressed", "true");
    expect(
      screen.getByRole("button", { name: /MCP servers/ }),
    ).toBeInTheDocument();

    await view.user.click(
      screen.getByRole("button", { name: /^trusted/, expanded: false }),
    );

    await view.user.click(
      screen.getByRole("switch", { name: "Send secrets to trusted" }),
    );

    expect(savedPolicy).toBeNull();

    await view.user.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => {
      expect(savedPolicy).not.toBeNull();
    });
    expect(savedPolicy).toEqual({
      ...policy,
      zones: [{ ...policy.zones[0], send_to: [] }, policy.zones[1]],
    });
  });

  it("limits a provider to a subset of MCP servers", async () => {
    let savedPolicy: PrivacyPolicy | null = null;
    mockPolicyEndpoints((saved) => {
      savedPolicy = saved;
    });

    const view = render(<PrivacySettingsSection />);

    await view.user.click(
      await screen.findByRole("button", { name: /^trusted/, expanded: false }),
    );

    const mcpSwitch = screen.getByRole("switch", {
      name: "Allow trusted to use build-mcp",
    });
    expect(mcpSwitch).toBeChecked();

    await view.user.click(mcpSwitch);

    await waitFor(() => {
      expect(savedPolicy).not.toBeNull();
    });
    expect(savedPolicy).toEqual({
      ...policy,
      tool_access: { providers: { trusted: { mcp: [] } } },
    });
  });

  it("edits globally blocked patterns", async () => {
    let savedPolicy: PrivacyPolicy | null = null;
    mockPolicyEndpoints((saved) => {
      savedPolicy = saved;
    });

    const view = render(<PrivacySettingsSection />);

    expect(await screen.findByText("*.blocked")).toBeInTheDocument();

    await view.user.click(
      screen.getByRole("button", { name: "Add blocked pattern" }),
    );
    await view.user.type(
      screen.getByRole("textbox", { name: "Add blocked pattern" }),
      "id_rsa{Enter}",
    );

    await waitFor(() => {
      expect(savedPolicy).not.toBeNull();
    });
    expect(savedPolicy).toEqual({
      ...policy,
      blocked: ["*.blocked", "id_rsa"],
    });
  });

  it("defers adding a zone until Save changes is clicked", async () => {
    const savedPolicies: PrivacyPolicy[] = [];
    mockPolicyEndpoints((saved) => savedPolicies.push(saved));

    const view = render(<PrivacySettingsSection />);

    await view.user.click(
      await screen.findByRole("button", { name: "Add zone" }),
    );

    expect(savedPolicies).toHaveLength(0);
    expect(
      screen.getByRole("textbox", { name: "Zone name for new_zone" }),
    ).toBeInTheDocument();

    await view.user.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(savedPolicies).toHaveLength(1));
    expect(savedPolicies[0]?.zones[0]).toEqual({
      name: "new_zone",
      patterns: [],
      send_to: [],
      on_shell_read: "withhold",
    });
  });

  it("defers zone name and pattern edits until Save changes is clicked", async () => {
    const savedPolicies: PrivacyPolicy[] = [];
    mockPolicyEndpoints((saved) => savedPolicies.push(saved));

    const view = render(<PrivacySettingsSection />);
    const name = await screen.findByRole("textbox", {
      name: "Zone name for secrets",
    });

    await view.user.clear(name);
    await view.user.type(name, "credentials{Enter}");
    await view.user.click(
      screen.getAllByRole("button", { name: "Add pattern" })[0],
    );
    await view.user.type(
      screen.getByRole("textbox", { name: "Add pattern" }),
      "*.pem{Enter}",
    );

    expect(savedPolicies).toHaveLength(0);

    await view.user.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(savedPolicies).toHaveLength(1));
    expect(savedPolicies[0]?.zones[0]).toEqual({
      ...policy.zones[0],
      name: "credentials",
      patterns: [".env*", "*.pem"],
    });
  });

  it("preserves an unsaved added zone during an immediate blocked-pattern save", async () => {
    const savedPolicies: PrivacyPolicy[] = [];
    mockPolicyEndpoints((saved) => savedPolicies.push(saved));

    const view = render(<PrivacySettingsSection />);

    await view.user.click(
      await screen.findByRole("button", { name: "Add zone" }),
    );
    await view.user.click(
      screen.getByRole("button", { name: "Add blocked pattern" }),
    );
    await view.user.type(
      screen.getByRole("textbox", { name: "Add blocked pattern" }),
      "id_rsa{Enter}",
    );

    await waitFor(() => expect(savedPolicies).toHaveLength(1));
    expect(savedPolicies[0]).toEqual({
      ...policy,
      blocked: ["*.blocked", "id_rsa"],
    });
    expect(
      screen.getByRole("textbox", { name: "Zone name for new_zone" }),
    ).toBeInTheDocument();

    await view.user.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(savedPolicies).toHaveLength(2));
    expect(savedPolicies[1]).toEqual({
      ...policy,
      blocked: ["*.blocked", "id_rsa"],
      zones: [
        {
          name: "new_zone",
          patterns: [],
          send_to: [],
          on_shell_read: "withhold",
        },
        ...policy.zones,
      ],
    });
  });

  it("keeps the access matrix collapsed until requested", async () => {
    mockPolicyEndpoints(() => undefined);

    const view = render(<PrivacySettingsSection />);

    const toggle = await screen.findByRole("button", { name: "Show matrix" });
    expect(
      screen.queryByRole("table", { name: "Zone destination permissions" }),
    ).not.toBeInTheDocument();

    await view.user.click(toggle);

    const matrix = await screen.findByRole("table", {
      name: "Zone destination permissions",
    });
    expect(within(matrix).getAllByRole("columnheader")).toHaveLength(3);
  });

  it("shows configuration and degraded-observation errors loudly", async () => {
    server.use(
      http.get("*/v1/privacy/policy", () =>
        HttpResponse.json({
          ...response,
          error: "privacy.yaml: malformed glob",
        } satisfies PrivacyPolicyResponse),
      ),
      http.get("*/v1/privacy/status", () =>
        HttpResponse.json({
          platform: "linux",
          observation: {
            platform_supported: true,
            runtime_available: false,
            last_error: "PTRACE_TRACEME is unavailable",
          },
          config_error: "privacy.yaml: malformed glob",
        } satisfies PrivacyStatusResponse),
      ),
    );

    render(<PrivacySettingsSection />);

    expect(
      await screen.findByRole("alert", {
        name: /Privacy configuration error/i,
      }),
    ).toHaveTextContent("privacy.yaml: malformed glob");
    expect(screen.getByText("Degraded attribution")).toBeInTheDocument();
    expect(
      screen.getByText("PTRACE_TRACEME is unavailable"),
    ).toBeInTheDocument();
  });

  it("does not claim runtime availability before an observation attempt", async () => {
    server.use(
      http.get("*/v1/privacy/policy", () => HttpResponse.json(response)),
      http.get("*/v1/privacy/status", () =>
        HttpResponse.json({
          platform: "linux",
          observation: {
            platform_supported: true,
            runtime_available: false,
            last_error: null,
          },
          config_error: null,
        } satisfies PrivacyStatusResponse),
      ),
    );

    render(<PrivacySettingsSection />);

    expect(await screen.findByText("Runtime unknown")).toBeInTheDocument();
    expect(
      screen.getByText(
        "No observation attempt has run yet. Runtime availability is unknown.",
      ),
    ).toBeInTheDocument();
  });

  it("shows that folder patterns are recursive", async () => {
    mockPolicyEndpoints(() => undefined);
    render(<PrivacySettingsSection />);
    expect(
      (
        await screen.findAllByText(
          "A folder path also covers everything inside it.",
        )
      ).length,
    ).toBeGreaterThan(0);
  });

  it("deletes a zone immediately after confirmation and keeps it deleted", async () => {
    const savedPolicies: PrivacyPolicy[] = [];
    mockPolicyEndpoints((saved) => savedPolicies.push(saved));

    const view = render(<PrivacySettingsSection />);
    await view.user.click(
      await screen.findByRole("button", { name: "Remove zone secrets" }),
    );

    const dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveTextContent('Remove zone "secrets"?');
    expect(dialog).toHaveTextContent("This takes effect immediately.");
    expect(savedPolicies).toHaveLength(0);

    await view.user.click(
      within(dialog).getByRole("button", { name: "Remove zone" }),
    );

    await waitFor(() => expect(savedPolicies).toHaveLength(1));
    expect(savedPolicies[0]).toEqual({ ...policy, zones: [policy.zones[1]] });
    await waitFor(() =>
      expect(
        screen.queryByRole("textbox", { name: "Zone name for secrets" }),
      ).not.toBeInTheDocument(),
    );
    expect(screen.getByRole("button", { name: "Save changes" })).toBeDisabled();

    view.unmount();
    render(<PrivacySettingsSection />, { store: view.store });
    expect(
      await screen.findByRole("textbox", { name: "Zone name for normal" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("textbox", { name: "Zone name for secrets" }),
    ).not.toBeInTheDocument();
  });

  it("does not save when zone deletion is cancelled", async () => {
    const savedPolicies: PrivacyPolicy[] = [];
    mockPolicyEndpoints((saved) => savedPolicies.push(saved));

    const view = render(<PrivacySettingsSection />);
    await view.user.click(
      await screen.findByRole("button", { name: "Remove zone secrets" }),
    );
    const dialog = await screen.findByRole("dialog");
    await view.user.click(
      within(dialog).getByRole("button", { name: "Cancel" }),
    );

    await waitFor(() =>
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
    );
    expect(savedPolicies).toHaveLength(0);
    expect(
      screen.getByRole("textbox", { name: "Zone name for secrets" }),
    ).toBeInTheDocument();
  });

  it("keeps other unsaved zone drafts when a zone is deleted", async () => {
    const threeZones: PrivacyPolicy = {
      ...policy,
      zones: [
        policy.zones[0],
        {
          name: "internal",
          patterns: ["docs/"],
          send_to: [],
          on_shell_read: "withhold",
        },
        policy.zones[1],
      ],
    };
    const savedPolicies: PrivacyPolicy[] = [];
    let currentPolicy = threeZones;
    server.use(
      http.get("*/v1/privacy/policy", () =>
        HttpResponse.json({ ...response, policy: currentPolicy }),
      ),
      http.get("*/v1/privacy/status", () => HttpResponse.json(status)),
      http.post("*/v1/privacy/policy", async ({ request }) => {
        const saved = (await request.json()) as PrivacyPolicy;
        savedPolicies.push(saved);
        currentPolicy = saved;
        return HttpResponse.json({ ...response, policy: saved });
      }),
    );

    const view = render(<PrivacySettingsSection />);
    const name = await screen.findByRole("textbox", {
      name: "Zone name for internal",
    });
    await view.user.clear(name);
    await view.user.type(name, "docs{Enter}");
    await view.user.click(screen.getByRole("button", { name: "Add zone" }));

    await view.user.click(
      screen.getByRole("button", { name: "Remove zone secrets" }),
    );
    await view.user.click(
      within(await screen.findByRole("dialog")).getByRole("button", {
        name: "Remove zone",
      }),
    );

    await waitFor(() => expect(savedPolicies).toHaveLength(1));
    expect(savedPolicies[0]).toEqual({
      ...threeZones,
      zones: [threeZones.zones[1], threeZones.zones[2]],
    });

    await waitFor(() =>
      expect(
        screen.queryByRole("textbox", { name: "Zone name for secrets" }),
      ).not.toBeInTheDocument(),
    );
    expect(
      screen.getByRole("textbox", { name: "Zone name for docs" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("textbox", { name: "Zone name for new_zone" }),
    ).toBeInTheDocument();
    const save = screen.getByRole("button", { name: "Save changes" });
    expect(save).toBeEnabled();

    await view.user.click(save);
    await waitFor(() => expect(savedPolicies).toHaveLength(2));
    expect(savedPolicies[1]?.zones.map((zone) => zone.name)).toEqual([
      "new_zone",
      "docs",
      "normal",
    ]);
  });

  it("keeps the zone and shows the error when deletion fails", async () => {
    const savedPolicies: PrivacyPolicy[] = [];
    mockPolicyEndpoints((saved) => savedPolicies.push(saved), {
      failSave: true,
    });

    const view = render(<PrivacySettingsSection />);
    await view.user.click(
      await screen.findByRole("button", { name: "Remove zone secrets" }),
    );
    await view.user.click(
      within(await screen.findByRole("dialog")).getByRole("button", {
        name: "Remove zone",
      }),
    );

    expect(
      await screen.findByText("Privacy policy was not saved"),
    ).toBeInTheDocument();
    expect(screen.getByText("privacy.yaml is read-only")).toBeInTheDocument();
    expect(savedPolicies).toHaveLength(1);
    expect(
      screen.getByRole("textbox", { name: "Zone name for secrets" }),
    ).toBeInTheDocument();
  });

  it("refetches destinations after an integration is saved", async () => {
    let destinations = response.destinations;
    let policyFetches = 0;
    server.use(
      http.get("*/v1/privacy/policy", () => {
        policyFetches += 1;
        return HttpResponse.json({ ...response, destinations });
      }),
      http.get("*/v1/privacy/status", () => HttpResponse.json(status)),
      http.post("*/v1/integration-save", () => HttpResponse.json({})),
    );

    const view = render(<PrivacySettingsSection />);
    await screen.findByRole("button", { name: /^trusted/, expanded: false });
    expect(
      screen.queryByRole("button", { name: /^http_unreal/ }),
    ).not.toBeInTheDocument();
    const fetchesBefore = policyFetches;

    destinations = [
      ...response.destinations,
      { id: "http_unreal", kind: "provider", display_name: "http_unreal" },
    ];
    await view.store.dispatch(
      integrationsApi.endpoints.saveIntegration.initiate({
        filePath: "/home/user/.config/refact/integrations.d/http_unreal.yaml",
        values: {},
      }),
    );

    await view.user.click(
      await screen.findByRole("button", {
        name: /^http_unreal/,
        expanded: false,
      }),
    );
    expect(policyFetches).toBeGreaterThan(fetchesBefore);
    expect(
      screen.getByRole("switch", { name: "Send secrets to http_unreal" }),
    ).toBeInTheDocument();
  });

  it("refetches the privacy policy after an MCP marketplace install", async () => {
    let policyFetches = 0;
    server.use(
      http.get("*/v1/privacy/policy", () => {
        policyFetches += 1;
        return HttpResponse.json(response);
      }),
      http.get("*/v1/privacy/status", () => HttpResponse.json(status)),
      http.post("*/v1/mcp/marketplace/install", () =>
        HttpResponse.json({ config_path: "/tmp/mcp_x.yaml" }),
      ),
    );

    const view = render(<PrivacySettingsSection />);
    await screen.findByRole("button", { name: "Add zone" });
    const fetchesBefore = policyFetches;

    await view.store.dispatch(
      mcpMarketplaceApi.endpoints.installServer.initiate({ server_id: "x" }),
    );

    await waitFor(() => expect(policyFetches).toBeGreaterThan(fetchesBefore));
  });

  it("refetches on mount even when a policy is cached", async () => {
    let policyFetches = 0;
    server.use(
      http.get("*/v1/privacy/policy", () => {
        policyFetches += 1;
        return HttpResponse.json(response);
      }),
      http.get("*/v1/privacy/status", () => HttpResponse.json(status)),
    );

    const first = render(<PrivacySettingsSection />);
    await screen.findByRole("button", { name: "Add zone" });
    expect(policyFetches).toBe(1);
    first.unmount();

    render(<PrivacySettingsSection />, { store: first.store });
    await waitFor(() => expect(policyFetches).toBe(2));
  });

  it("polls again while destination discovery is deferred", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    let policyFetches = 0;
    server.use(
      http.get("*/v1/privacy/policy", () => {
        policyFetches += 1;
        return HttpResponse.json({
          ...response,
          discovery_deferred: policyFetches < 2,
        });
      }),
      http.get("*/v1/privacy/status", () => HttpResponse.json(status)),
    );

    render(<PrivacySettingsSection />);
    await screen.findByRole("button", { name: "Add zone" });
    expect(policyFetches).toBe(1);

    await vi.advanceTimersByTimeAsync(3000);
    await waitFor(() => expect(policyFetches).toBe(2));

    await vi.advanceTimersByTimeAsync(6000);
    expect(policyFetches).toBe(2);
  });

  it("keeps polling after a rejected discovery refetch", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    let policyFetches = 0;
    server.use(
      http.get("*/v1/privacy/policy", () => {
        policyFetches += 1;
        if (policyFetches === 2) {
          return HttpResponse.json({ detail: "boom" }, { status: 500 });
        }
        return HttpResponse.json({
          ...response,
          discovery_deferred: policyFetches < 3,
        });
      }),
      http.get("*/v1/privacy/status", () => HttpResponse.json(status)),
    );

    render(<PrivacySettingsSection />);
    await screen.findByRole("button", { name: "Add zone" });
    expect(policyFetches).toBe(1);

    await advanceUntil(() => policyFetches >= 2);
    expect(policyFetches).toBe(2);

    await advanceUntil(() => policyFetches >= 3);
    expect(policyFetches).toBe(3);

    await vi.advanceTimersByTimeAsync(9000);
    expect(policyFetches).toBe(3);
  });

  it("stops polling at the retry cap and polls again for a new episode", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    let policyFetches = 0;
    server.use(
      http.get("*/v1/privacy/policy", () => {
        policyFetches += 1;
        return HttpResponse.json({ ...response, discovery_deferred: true });
      }),
      http.get("*/v1/privacy/status", () => HttpResponse.json(status)),
      http.post("*/v1/integration-save", () => HttpResponse.json({})),
    );

    const view = render(<PrivacySettingsSection />);
    await screen.findByRole("button", { name: "Add zone" });
    expect(policyFetches).toBe(1);

    await advanceUntil(() => policyFetches >= 11, 200);
    expect(policyFetches).toBe(11);

    await vi.advanceTimersByTimeAsync(30000);
    expect(policyFetches).toBe(11);

    await view.store.dispatch(
      integrationsApi.endpoints.saveIntegration.initiate({
        filePath: "/home/user/.config/refact/integrations.d/http_unreal.yaml",
        values: {},
      }),
    );
    await waitFor(() => expect(policyFetches).toBe(12));

    await advanceUntil(() => policyFetches >= 13);
    expect(policyFetches).toBe(13);
    await advanceUntil(() => policyFetches >= 14);
    expect(policyFetches).toBe(14);
  });

  it("schedules no discovery polling after unmount", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    let policyFetches = 0;
    server.use(
      http.get("*/v1/privacy/policy", () => {
        policyFetches += 1;
        return HttpResponse.json({ ...response, discovery_deferred: true });
      }),
      http.get("*/v1/privacy/status", () => HttpResponse.json(status)),
    );

    const view = render(<PrivacySettingsSection />);
    await screen.findByRole("button", { name: "Add zone" });
    expect(policyFetches).toBe(1);

    view.unmount();
    await vi.advanceTimersByTimeAsync(30000);
    expect(policyFetches).toBe(1);
  });

  it("deletes the original server zone when a renamed unsaved zone is removed", async () => {
    const savedPolicies: PrivacyPolicy[] = [];
    mockPolicyEndpoints((saved) => savedPolicies.push(saved));

    const view = render(<PrivacySettingsSection />);
    const name = await screen.findByRole("textbox", {
      name: "Zone name for secrets",
    });
    await view.user.clear(name);
    await view.user.type(name, "credentials{Enter}");
    await view.user.click(
      await screen.findByRole("button", { name: "Remove zone credentials" }),
    );

    const dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveTextContent('Remove zone "credentials"?');
    expect(dialog).toHaveTextContent("This takes effect immediately.");
    await view.user.click(
      within(dialog).getByRole("button", { name: "Remove zone" }),
    );

    await waitFor(() => expect(savedPolicies).toHaveLength(1));
    expect(savedPolicies[0]).toEqual({ ...policy, zones: [policy.zones[1]] });
    await waitFor(() =>
      expect(
        screen.queryByRole("textbox", { name: "Zone name for credentials" }),
      ).not.toBeInTheDocument(),
    );
    expect(
      screen.getByRole("textbox", { name: "Zone name for normal" }),
    ).toBeInTheDocument();
  });

  it("discards a never-saved zone locally without a request", async () => {
    const savedPolicies: PrivacyPolicy[] = [];
    mockPolicyEndpoints((saved) => savedPolicies.push(saved));

    const view = render(<PrivacySettingsSection />);
    await view.user.click(
      await screen.findByRole("button", { name: "Add zone" }),
    );
    await view.user.click(
      screen.getByRole("button", { name: "Remove zone new_zone" }),
    );

    const dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveTextContent('Remove unsaved zone "new_zone"?');
    expect(dialog).not.toHaveTextContent("takes effect immediately");
    await view.user.click(
      within(dialog).getByRole("button", { name: "Remove zone" }),
    );

    await waitFor(() =>
      expect(
        screen.queryByRole("textbox", { name: "Zone name for new_zone" }),
      ).not.toBeInTheDocument(),
    );
    expect(savedPolicies).toHaveLength(0);
    expect(screen.getByRole("button", { name: "Save changes" })).toBeDisabled();
  });
});
