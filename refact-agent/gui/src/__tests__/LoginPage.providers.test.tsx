import { describe, expect, test } from "vitest";
import { http, HttpResponse } from "msw";
import { setUpStore } from "../app/store";
import { STUB_CAPS_RESPONSE } from "../__fixtures__/caps";
import { LoginPage } from "../features/Login/LoginPage";
import { updateConfig } from "../features/Config/configSlice";
import { setBackendStatus } from "../features/Connection";
import { capsApi, providersApi } from "../services/refact";
import { server } from "../utils/mockServer";
import { render, screen, waitFor } from "../utils/test-utils";

async function unmountLogin(view: Awaited<ReturnType<typeof startLogin>>) {
  view.unmount();
  await Promise.all(
    view.store.dispatch(providersApi.util.getRunningMutationsThunk()),
  );
}

const provider = {
  name: "claude_code",
  base_provider: "claude_code",
  display_name: "Claude Code",
  enabled: true,
  readonly: false,
  has_credentials: false,
  status: "configured",
  model_count: 0,
};
const schema = `
fields:
  backend:
    f_type: string
    f_default: bedrock
oauth:
  supported: true
`;

function mockSetup() {
  server.use(
    http.get("*/v1/ping", () => HttpResponse.text("pong")),
    http.get("*/v1/caps", () => HttpResponse.json(STUB_CAPS_RESPONSE)),
    http.get("*/v1/providers", () =>
      HttpResponse.json({ providers: [provider] }),
    ),
    http.get("*/v1/providers/claude_code", () =>
      HttpResponse.json({
        ...provider,
        settings: { backend: "bedrock" },
        runtime: null,
      }),
    ),
    http.get("*/v1/providers/claude_code/schema", () =>
      HttpResponse.json({ name: "claude_code", schema }),
    ),
    http.get("*/v1/providers/claude_code/available-models", () =>
      HttpResponse.json({ models: [], source: "model_caps" }),
    ),
    http.post("*/v1/providers/claude_code/oauth/start", () =>
      HttpResponse.json({
        session_id: "bedrock-1",
        authorize_url: "",
        mode: "native",
        poll_interval: 1,
        instructions: "Finish AWS sign-in on the engine host",
      }),
    ),
    http.post("*/v1/providers/claude_code/oauth/logout", () =>
      HttpResponse.json({ success: true }),
    ),
  );
}

async function startLogin() {
  const store = setUpStore({
    config: { apiKey: "test", lspPort: 8001, themeProps: {}, host: "vscode" },
  });
  store.dispatch(
    updateConfig({ backendReady: true, connectionStatus: "ready" }),
  );
  store.dispatch(setBackendStatus({ status: "online" }));
  const view = render(<LoginPage />, { store });
  await view.user.click(
    await screen.findByRole("button", { name: /Claude Code/ }),
  );
  await view.user.click(await screen.findByRole("button", { name: "Login" }));
  await screen.findByText("Finish AWS sign-in on the engine host");
  return { ...view, store };
}

describe("LoginPage Bedrock sign-in lifecycle", () => {
  test("keeps polling the selected provider through pending responses", async () => {
    mockSetup();
    let polls = 0;
    server.use(
      http.post("*/v1/providers/claude_code/oauth/exchange", () => {
        polls++;
        return HttpResponse.json({
          success: false,
          status: "pending",
          auth_status: "Waiting for AWS",
          output: `AWS device prompt ${polls}`,
          poll_interval: 1,
        });
      }),
    );
    const view = await startLogin();
    const { store, user } = view;
    await screen.findByText("AWS device prompt 1", {}, { timeout: 3000 });
    await screen.findByText("AWS device prompt 2", {}, { timeout: 3000 });
    expect(screen.getByRole("button", { name: "Cancel" })).toBeEnabled();
    expect(
      screen.getByRole("button", { name: "Back to providers" }),
    ).toBeInTheDocument();
    expect(
      capsApi.endpoints.getCaps.select(undefined)(store.getState()).isSuccess,
    ).toBe(true);
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    await screen.findByRole("button", { name: "Login" });
    await unmountLogin(view);
  });

  test.each([200, 400])(
    "retains terminal Bedrock diagnostics for HTTP %s",
    async (status) => {
      mockSetup();
      server.use(
        http.post("*/v1/providers/claude_code/oauth/exchange", () => {
          return HttpResponse.json(
            status === 200
              ? {
                  success: false,
                  status: "failed",
                  auth_status: "AWS SSO profile is missing",
                  output: "Configure the AWS SSO profile first",
                }
              : { detail: "AWS SSO profile is missing" },
            { status },
          );
        }),
      );
      const view = await startLogin();
      expect(
        await screen.findByText(
          "AWS SSO profile is missing",
          {},
          { timeout: 3000 },
        ),
      ).toHaveAttribute("role", "alert");
      if (status === 200)
        expect(
          screen.getByText("Configure the AWS SSO profile first"),
        ).toBeInTheDocument();
      await waitFor(() =>
        expect(screen.getByRole("button", { name: "Try again" })).toBeEnabled(),
      );
      expect(screen.getByText("AWS SSO profile is missing")).toBeVisible();
      expect(
        screen.getByRole("button", { name: "Back to providers" }),
      ).toBeInTheDocument();
      await unmountLogin(view);
    },
  );
});
