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
};

const response: PrivacyPolicyResponse = {
  policy,
  destinations: [
    {
      id: "trusted",
      kind: "provider",
      display_name: "Trusted provider",
    },
    {
      id: "build-mcp",
      kind: "mcp",
      display_name: "Build MCP",
    },
  ],
  match_counts: { secrets: 2, normal: 7 },
  error: null,
  source_paths: ["/home/user/.config/refact/privacy.yaml"],
};

const status: PrivacyStatusResponse = {
  platform: "linux",
  observation: { available: true, reason: null },
  config_error: null,
};

describe("PrivacySettingsSection", () => {
  it("renders one destination column per destination and saves a cell toggle", async () => {
    let savedPolicy: PrivacyPolicy | null = null;
    server.use(
      http.get("*/v1/privacy/policy", () => HttpResponse.json(response)),
      http.get("*/v1/privacy/status", () => HttpResponse.json(status)),
      http.post("*/v1/privacy/policy", async ({ request }) => {
        savedPolicy = (await request.json()) as PrivacyPolicy;
        return HttpResponse.json({
          ...response,
          policy: savedPolicy,
        } satisfies PrivacyPolicyResponse);
      }),
    );

    const view = render(<PrivacySettingsSection />);
    const grid = await screen.findByRole("table", {
      name: "Zone destination permissions",
    });
    const columns = within(grid).getAllByRole("columnheader");

    expect(columns).toHaveLength(3);
    expect(columns[1]).toHaveTextContent("Trusted provider");
    expect(columns[2]).toHaveTextContent("Build MCP");
    expect(within(grid).getByText("2 matches")).toBeInTheDocument();
    expect(within(grid).getByText("7 matches")).toBeInTheDocument();
    expect(screen.getByText("Available on linux")).toBeInTheDocument();
    expect(
      screen.getByRole("combobox", {
        name: "Shell read behavior for secrets",
      }),
    ).toBeInTheDocument();

    await view.user.click(
      within(grid).getByRole("button", {
        name: "Allow secrets to Build MCP",
      }),
    );

    await waitFor(() => {
      expect(savedPolicy).not.toBeNull();
    });
    expect(savedPolicy).toEqual({
      ...policy,
      zones: [
        {
          ...policy.zones[0],
          send_to: ["trusted", "build-mcp"],
        },
        policy.zones[1],
      ],
    });
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
          platform: "windows",
          observation: {
            available: false,
            reason: "File observation is unavailable on this platform.",
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
      screen.getByText("File observation is unavailable on this platform."),
    ).toBeInTheDocument();
  });
});
