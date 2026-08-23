import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "../../../utils/test-utils";
import { http, HttpResponse } from "msw";

import { server } from "../../../utils/mockServer";
import { ProviderForm, type ProviderListItem } from "./ProviderForm";

const provider: ProviderListItem = {
  name: "litellm_team",
  base_provider: "litellm",
  display_name: "Team LiteLLM",
  enabled: true,
  readonly: false,
  has_credentials: false,
  status: "active",
  model_count: 1,
};

const schema = `
description: LiteLLM gateway settings
fields:
  endpoint:
    f_type: string
    f_label: Endpoint
  api_key:
    f_type: string
    f_label: API Key
    f_secret: true
  credential:
    f_type: string_long
    f_label: Credential Command
    f_object: true
    f_confirmation: true
  supports_cache_control:
    f_type: boolean
    f_label: Cache Control
  enabled:
    f_type: boolean
    f_label: Enable Provider
  extra_headers:
    f_type: string_long
    f_label: Extra Headers
    f_object: true
    f_extra: true
`;

const preloadedState = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "vscode" as const,
  },
};

describe("LiteLLM ProviderForm contract", () => {
  it("renders typed gateway settings and preserves provider identity on updates", async () => {
    const updates: unknown[] = [];
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);

    server.use(
      http.get("*/v1/providers", () =>
        HttpResponse.json({ providers: [provider] }),
      ),
      http.get("*/v1/providers/litellm_team", () =>
        HttpResponse.json({
          ...provider,
          selected_models_count: 1,
          settings: {
            endpoint: "https://gateway.example.com/v1",
            api_key: "***",
            credential: {
              type: "command",
              command: "secret-helper",
              args: ["litellm"],
            },
            supports_cache_control: true,
            enabled: false,
            extra_headers: { "X-Team": "platform" },
          },
          runtime: null,
        }),
      ),
      http.get("*/v1/providers/litellm_team/schema", () =>
        HttpResponse.json({ name: "litellm_team", schema }),
      ),
      http.post("*/v1/providers/litellm_team", async ({ request }) => {
        updates.push(await request.json());
        return HttpResponse.json({ success: true });
      }),
    );

    const { user } = render(<ProviderForm currentProvider={provider} />, {
      preloadedState,
    });

    expect(
      await screen.findByDisplayValue("https://gateway.example.com/v1"),
    ).toBeInTheDocument();
    const apiKey = screen.getByLabelText("API Key");
    expect(apiKey).toHaveAttribute("type", "password");
    expect(apiKey).toHaveAttribute("placeholder", "••••••••  (saved)");

    const command = screen.getByLabelText("Credential Command");
    expect(command).toHaveValue(
      "args:\n  - litellm\ncommand: secret-helper\ntype: command",
    );
    fireEvent.change(command, {
      target: { value: 'command: token-helper\nargs: ["team"]' },
    });
    fireEvent.blur(command);

    expect(screen.getByLabelText("Cache Control")).toBeChecked();
    const enabled = screen.getByLabelText("Enable Provider");
    expect(enabled).not.toBeChecked();
    await user.click(enabled);

    await user.click(
      screen.getByRole("button", { name: "Show advanced fields" }),
    );
    expect(screen.getByLabelText("Extra Headers")).toHaveValue(
      "X-Team: platform",
    );

    await waitFor(() => {
      expect(confirm).toHaveBeenCalledWith(
        "Save changes to Credential Command?",
      );
      expect(updates).toContainEqual({
        base_provider: "litellm",
        display_name: "Team LiteLLM",
        credential: { command: "token-helper", args: ["team"] },
      });
      expect(updates).toContainEqual({
        base_provider: "litellm",
        display_name: "Team LiteLLM",
        enabled: true,
      });
    });

    confirm.mockRestore();
  });
});
