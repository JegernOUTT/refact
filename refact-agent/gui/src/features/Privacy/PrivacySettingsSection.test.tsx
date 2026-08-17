import { HttpResponse, http } from "msw";
import { describe, expect, it } from "vitest";

import type {
  PrivacyPolicy,
  PrivacyPolicyResponse,
  PrivacyStatusResponse,
} from "../../services/refact/privacy";
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

function mockPolicyEndpoints(onSave: (policy: PrivacyPolicy) => void) {
  server.use(
    http.get("*/v1/privacy/policy", () => HttpResponse.json(response)),
    http.get("*/v1/privacy/status", () => HttpResponse.json(status)),
    http.post("*/v1/privacy/policy", async ({ request }) => {
      const saved = (await request.json()) as PrivacyPolicy;
      onSave(saved);
      return HttpResponse.json({
        ...response,
        policy: saved,
      } satisfies PrivacyPolicyResponse);
    }),
  );
}

describe("PrivacySettingsSection", () => {
  it("groups destinations by kind and saves a zone toggle from a destination row", async () => {
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
});
