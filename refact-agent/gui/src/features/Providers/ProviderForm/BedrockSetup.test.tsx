import { describe, expect, it, vi } from "vitest";
import { http, HttpResponse } from "msw";
import { act } from "react-dom/test-utils";
import { render, screen, waitFor, fireEvent } from "../../../utils/test-utils";
import { server } from "../../../utils/mockServer";
import { BedrockSetup } from "./BedrockSetup";

const preloadedState = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "vscode" as const,
  },
};
const endpoint = "*/v1/providers/team/bedrock";
function mockSetup() {
  const requests: {
    path: string;
    body: unknown;
    authorization: string | null;
  }[] = [];
  server.use(
    http.get(`${endpoint}/profiles`, () =>
      HttpResponse.json({
        profiles: [{ name: "work", region: "eu-west-1", sso_configured: true }],
        suggested_profile: "work",
        suggested_region: "eu-west-1",
      }),
    ),
    http.post(`${endpoint}/verify`, async ({ request }) => {
      requests.push({
        path: "verify",
        body: await request.json(),
        authorization: request.headers.get("Authorization"),
      });
      return HttpResponse.json({
        setup_id: "setup",
        config_revision: "revision",
        profile: "work",
        region: "eu-west-1",
        identity: {
          account: "123",
          arn: "arn:aws:iam::123:role/team",
          user_id: "user",
        },
        inference_profiles: [],
        candidates: [
          { model_id: "good", supports_1m: true },
          { model_id: "denied" },
        ],
        warnings: [],
      });
    }),
    http.post(`${endpoint}/probe`, async ({ request }) => {
      requests.push({
        path: "probe",
        body: await request.json(),
        authorization: request.headers.get("Authorization"),
      });
      return HttpResponse.json({
        setup_id: "setup",
        results: [
          { model_id: "good", status: "working" },
          {
            model_id: "denied",
            status: "denied",
            message: "Missing InvokeModel permission",
          },
        ],
        warnings: [],
      });
    }),
    http.post(`${endpoint}/apply`, async ({ request }) => {
      requests.push({
        path: "apply",
        body: await request.json(),
        authorization: request.headers.get("Authorization"),
      });
      return HttpResponse.json({ success: true, model_ids: ["good"] });
    }),
  );
  return requests;
}

describe("Bedrock guided setup", () => {
  it("ignores verification finishing after provider settings change", async () => {
    mockSetup();
    let finish: (() => void) | undefined;
    const pending = new Promise<void>((resolve) => {
      finish = resolve;
    });
    server.use(
      http.post(`${endpoint}/verify`, async () => {
        await pending;
        return HttpResponse.json({
          setup_id: "old",
          config_revision: "old",
          profile: "work",
          region: "eu-west-1",
          identity: { account: "old-account", arn: "old-arn", user_id: "old" },
          inference_profiles: [],
          candidates: [],
          warnings: [],
        });
      }),
    );
    const save = vi.fn();
    const { user, rerender } = render(
      <BedrockSetup
        providerName="team"
        readonly={false}
        settingsRevision="old"
        onSaveRecovery={save}
      />,
      { preloadedState },
    );
    await screen.findByDisplayValue("work");
    await user.click(
      screen.getByRole("button", {
        name: "Verify identity and discover models",
      }),
    );
    rerender(
      <BedrockSetup
        providerName="team"
        readonly={false}
        settingsRevision="new"
        onSaveRecovery={save}
      />,
    );
    finish?.();
    await waitFor(() =>
      expect(
        screen.getByRole("button", {
          name: "Verify identity and discover models",
        }),
      ).toBeEnabled(),
    );
    expect(screen.queryByText(/old-account/)).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Test models" }),
    ).not.toBeInTheDocument();
  });
  it("discovers profiles without invoking AWS, explicitly tests then pins only working IDs", async () => {
    const requests = mockSetup();
    const { user } = render(
      <BedrockSetup
        providerName="team"
        readonly={false}
        onSaveRecovery={vi.fn()}
      />,
      { preloadedState },
    );
    expect(await screen.findByDisplayValue("work")).toBeInTheDocument();
    expect(screen.getByLabelText("AWS region")).toHaveValue("eu-west-1");
    expect(requests).toEqual([]);
    await user.click(
      screen.getByRole("button", {
        name: "Verify identity and discover models",
      }),
    );
    expect(await screen.findByText(/Verified account 123/)).toBeInTheDocument();
    expect(screen.getByText(/AWS may charge/)).toBeInTheDocument();
    expect(
      screen.queryByLabelText("Enable 1M context"),
    ).not.toBeInTheDocument();
    expect(requests.map((r) => r.path)).toEqual(["verify"]);
    await user.click(screen.getByRole("button", { name: "Test models" }));
    expect(await screen.findByText(/Permission denied/)).toBeInTheDocument();
    await user.click(
      screen.getByRole("button", { name: "Pin working model IDs" }),
    );
    expect(
      await screen.findByText(/Working model IDs pinned/),
    ).toBeInTheDocument();
    expect(requests).toEqual([
      {
        path: "verify",
        body: { profile: "work", region: "eu-west-1" },
        authorization: "Bearer test",
      },
      {
        path: "probe",
        body: {
          setup_id: "setup",
          model_ids: ["good", "denied"],
          context_1m: false,
        },
        authorization: "Bearer test",
      },
      {
        path: "apply",
        body: {
          setup_id: "setup",
          config_revision: "revision",
          model_ids: ["good"],
          context_1m: false,
        },
        authorization: "Bearer test",
      },
    ]);
  });

  it("skip preserves settings and defaults; readonly prevents verification and tests", async () => {
    const requests = mockSetup();
    const save = vi.fn();
    const { user } = render(
      <BedrockSetup providerName="team" readonly onSaveRecovery={save} />,
      { preloadedState },
    );
    await screen.findByDisplayValue("work");
    expect(
      screen.getByRole("button", {
        name: "Verify identity and discover models",
      }),
    ).toBeDisabled();
    await user.click(screen.getByRole("button", { name: "Skip setup" }));
    expect(
      screen.getByText(/Existing models and defaults are unchanged/),
    ).toBeInTheDocument();
    expect(requests).toEqual([]);
    expect(save).not.toHaveBeenCalled();
  });

  it("invalidates old verification when profile or custom IDs change and exposes stale setup errors", async () => {
    mockSetup();
    server.use(
      http.post(`${endpoint}/apply`, () =>
        HttpResponse.json(
          { detail: "Configuration changed. Verify again." },
          { status: 409 },
        ),
      ),
    );
    const { user } = render(
      <BedrockSetup
        providerName="team"
        readonly={false}
        onSaveRecovery={vi.fn()}
      />,
      { preloadedState },
    );
    await screen.findByDisplayValue("work");
    await user.click(
      screen.getByRole("button", {
        name: "Verify identity and discover models",
      }),
    );
    await screen.findByText(/Verified account/);
    await user.click(screen.getByLabelText("denied"));
    expect(screen.getByLabelText("Enable 1M context")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Test models" }));
    await screen.findByRole("button", { name: "Pin working model IDs" });
    await user.click(
      screen.getByRole("button", { name: "Pin working model IDs" }),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Configuration changed. Verify again.",
    );
    fireEvent.change(screen.getByLabelText("Alternative or custom model IDs"), {
      target: { value: "custom-id" },
    });
    expect(
      screen.queryByRole("button", { name: "Pin working model IDs" }),
    ).not.toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("AWS region"), {
      target: { value: "us-east-1" },
    });
    expect(screen.queryByText(/Verified account/)).not.toBeInTheDocument();
  });

  it("only exposes SSO after explicitly saving the selected profile and region", async () => {
    mockSetup();
    const save = vi.fn().mockResolvedValue(undefined);
    const { user } = render(
      <BedrockSetup
        providerName="team"
        readonly={false}
        onSaveRecovery={save}
        savedProfile="work"
        savedRegion="eu-west-1"
        recovery={<div>Existing SSO control</div>}
      />,
      { preloadedState },
    );
    await screen.findByDisplayValue("work");
    await user.click(screen.getByText("Optional AWS SSO recovery"));
    expect(save).not.toHaveBeenCalled();
    expect(screen.queryByText("Existing SSO control")).not.toBeInTheDocument();
    await user.click(
      screen.getByRole("button", {
        name: "Save selected profile for SSO recovery",
      }),
    );
    await waitFor(() => expect(save).toHaveBeenCalledWith("work", "eu-west-1"));
    expect(await screen.findByText("Existing SSO control")).toBeInTheDocument();
  });
  it.each(["profile", "settings", "readonly", "instance"])(
    "revokes delayed recovery after an external %s change",
    async (change) => {
      mockSetup();
      let finish!: () => void;
      const save = vi.fn(
        () =>
          new Promise<void>((resolve) => {
            finish = resolve;
          }),
      );
      const props = {
        providerName: "team",
        readonly: false,
        savedProfile: "old",
        savedRegion: "eu-west-1",
        settingsRevision: "old",
        recoverySettingsRevision: "stable",
        onSaveRecovery: save,
        recovery: <div>Existing SSO control</div>,
      };
      const { user, rerender } = render(<BedrockSetup {...props} />, {
        preloadedState,
      });
      fireEvent.change(screen.getByLabelText("AWS profile"), {
        target: { value: "work" },
      });
      fireEvent.change(screen.getByLabelText("AWS region"), {
        target: { value: "eu-west-1" },
      });
      await user.click(screen.getByText("Optional AWS SSO recovery"));
      await user.click(
        screen.getByRole("button", {
          name: "Save selected profile for SSO recovery",
        }),
      );
      await waitFor(() => expect(save).toHaveBeenCalled());
      const changed = {
        ...props,
        savedProfile: change === "profile" ? "other" : "work",
        settingsRevision: "new",
        recoverySettingsRevision: change === "settings" ? "changed" : "stable",
        readonly: change === "readonly",
        providerName: change === "instance" ? "other" : "team",
      };
      if (change === "instance")
        server.use(
          http.get("*/v1/providers/other/bedrock/profiles", () =>
            HttpResponse.json({ profiles: [] }),
          ),
        );
      rerender(<BedrockSetup {...changed} />);
      await act(async () => {
        finish();
        await Promise.resolve();
      });
      expect(
        screen.queryByText("Existing SSO control"),
      ).not.toBeInTheDocument();
      // Returning to the requested pair must not revive a revoked result.
      rerender(<BedrockSetup {...props} savedProfile="work" />);
      expect(
        screen.queryByText("Existing SSO control"),
      ).not.toBeInTheDocument();
    },
  );

  it.each([true, false])(
    "preserves its own saved-pair refetch (before completion: %s), then revokes external changes",
    async (refetchFirst) => {
      mockSetup();
      let finish!: () => void;
      const save = vi.fn(
        () =>
          new Promise<void>((resolve) => {
            finish = resolve;
          }),
      );
      const props = {
        providerName: "team",
        readonly: false,
        savedProfile: "old",
        savedRegion: "eu-west-1",
        settingsRevision: "old",
        recoverySettingsRevision: "stable",
        onSaveRecovery: save,
        recovery: <div>Existing SSO control</div>,
      };
      const { user, rerender } = render(<BedrockSetup {...props} />, {
        preloadedState,
      });
      await screen.findByDisplayValue("old");
      fireEvent.change(screen.getByLabelText("AWS profile"), {
        target: { value: "work" },
      });
      fireEvent.change(screen.getByLabelText("AWS region"), {
        target: { value: "eu-west-1" },
      });
      await user.click(screen.getByText("Optional AWS SSO recovery"));
      await user.click(
        screen.getByRole("button", {
          name: "Save selected profile for SSO recovery",
        }),
      );
      await waitFor(() => expect(save).toHaveBeenCalled());
      if (refetchFirst)
        rerender(
          <BedrockSetup
            {...props}
            savedProfile="work"
            settingsRevision="saved"
          />,
        );
      await act(async () => {
        finish();
        await Promise.resolve();
      });
      if (!refetchFirst) {
        expect(
          screen.queryByText("Existing SSO control"),
        ).not.toBeInTheDocument();
        rerender(
          <BedrockSetup
            {...props}
            savedProfile="work"
            settingsRevision="saved"
          />,
        );
      }
      expect(
        await screen.findByText("Existing SSO control"),
      ).toBeInTheDocument();
      rerender(
        <BedrockSetup
          {...props}
          savedProfile="other"
          settingsRevision="external"
        />,
      );
      expect(
        screen.queryByText("Existing SSO control"),
      ).not.toBeInTheDocument();
    },
  );

  it("selects only eight discovered models and allows bounded alternatives", async () => {
    mockSetup();
    server.use(
      http.post(`${endpoint}/verify`, () =>
        HttpResponse.json({
          setup_id: "setup",
          config_revision: "revision",
          profile: "work",
          region: "eu-west-1",
          identity: { account: "123", arn: "arn", user_id: "user" },
          inference_profiles: [],
          warnings: [],
          candidates: Array.from({ length: 31 }, (_, i) => ({
            model_id: `model-${i}`,
          })),
        }),
      ),
    );
    const { user } = render(
      <BedrockSetup
        providerName="team"
        readonly={false}
        onSaveRecovery={vi.fn()}
      />,
      { preloadedState },
    );
    await screen.findByDisplayValue("work");
    await user.click(
      screen.getByRole("button", {
        name: "Verify identity and discover models",
      }),
    );
    await screen.findByText(/Verified account/);
    expect(screen.getAllByRole("switch", { checked: true })).toHaveLength(8);
    expect(screen.getByRole("button", { name: "Test models" })).toBeEnabled();
    await user.click(screen.getByLabelText("model-30"));
    expect(screen.getByRole("button", { name: "Test models" })).toBeDisabled();
    await user.click(screen.getByLabelText("model-0"));
    expect(screen.getByRole("button", { name: "Test models" })).toBeEnabled();
    fireEvent.change(screen.getByLabelText("Alternative or custom model IDs"), {
      target: { value: "custom" },
    });
    expect(screen.getByRole("button", { name: "Test models" })).toBeDisabled();
  });
});
