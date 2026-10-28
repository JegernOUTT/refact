import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { http, HttpResponse } from "msw";
import { render, screen, waitFor } from "../utils/test-utils";
import { server } from "../utils/mockServer";
import { ProviderOAuth } from "../features/Providers/ProviderForm/ProviderOAuth";

const openSpy = vi.spyOn(window, "open").mockImplementation(() => null);

const PRELOADED_STATE = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "vscode" as const,
  },
};

function renderProviderOAuth(providerName: string) {
  return render(
    <ProviderOAuth
      providerName={providerName}
      oauthConnected={false}
      authStatus="No credentials found"
    />,
    { preloadedState: PRELOADED_STATE },
  );
}

function mockProviderOauthStart(providerName: string, body: object) {
  server.use(
    http.post(`*/v1/providers/${providerName}/oauth/start`, () =>
      HttpResponse.json(body),
    ),
  );
}

describe("ProviderOAuth", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    openSpy.mockImplementation(() => null);
  });

  afterEach(() => {
    openSpy.mockImplementation(() => null);
  });

  test("renders GitHub Copilot login label", () => {
    renderProviderOAuth("github_copilot");

    expect(screen.getByText("Login with GitHub Copilot")).toBeInTheDocument();
  });

  test("renders GitHub Copilot device flow fields after OAuth start", async () => {
    mockProviderOauthStart("github_copilot", {
      session_id: "github-session",
      authorize_url: "https://github.com/login/device",
      user_code: "ABCD-EFGH",
      instructions: "Enter code: ABCD-EFGH",
      poll_interval: 60,
      mode: "device",
    });

    const { user } = renderProviderOAuth("github_copilot");

    await user.click(screen.getByRole("button", { name: "Login" }));

    await waitFor(() => {
      expect(screen.getByText("ABCD-EFGH")).toBeInTheDocument();
    });
    expect(screen.getByText("Enter code: ABCD-EFGH")).toBeInTheDocument();
    expect(
      screen.getByText("https://github.com/login/device"),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Open verification page" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Retry" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeInTheDocument();
    expect(screen.getByText(/Checking every 60 seconds/i)).toBeInTheDocument();
  });

  test("renders Claude Code auto-callback flow after OAuth start", async () => {
    mockProviderOauthStart("claude_code", {
      session_id: "claude-session",
      authorize_url: "https://claude.ai/oauth/authorize",
      mode: "callback",
    });

    const { user } = renderProviderOAuth("claude_code");

    await user.click(screen.getByRole("button", { name: "Login" }));

    await waitFor(() => {
      expect(
        screen.getByText("Waiting for authentication..."),
      ).toBeInTheDocument();
    });
    expect(screen.queryByText("Paste the authorization code")).toBeNull();
  });

  test("renders OpenAI Codex auto-callback flow after OAuth start", async () => {
    mockProviderOauthStart("openai_codex", {
      session_id: "codex-session",
      authorize_url: "https://auth.openai.com/oauth/authorize",
    });

    const { user } = renderProviderOAuth("openai_codex");

    await user.click(screen.getByRole("button", { name: "Login" }));

    await waitFor(() => {
      expect(
        screen.getByText("Waiting for authentication..."),
      ).toBeInTheDocument();
    });
    expect(screen.queryByText("Paste the authorization code")).toBeNull();
  });
});

type NativeHarnessProps = {
  providerName?: string;
  backend: string;
  readonly?: boolean;
};

function renderNative({
  providerName = "claude_code",
  backend,
  readonly,
}: NativeHarnessProps) {
  return render(
    <ProviderOAuth
      providerName={providerName}
      baseProvider="claude_code"
      oauthConnected={false}
      authStatus=""
      loginKind="native"
      backend={backend}
      readonly={readonly}
    />,
    { preloadedState: PRELOADED_STATE },
  );
}

type Recorded = {
  starts: { provider: string; body: unknown }[];
  exchanges: { provider: string; body: unknown }[];
  logouts: { provider: string; body: unknown }[];
};

function mockNativeFlow(
  exchangeResponses: object[],
  startBody: object = {
    session_id: "native-1",
    authorize_url: "",
    mode: "native",
    instructions: "Finish signing in on the engine host.",
    poll_interval: 1,
  },
): Recorded {
  const recorded: Recorded = { starts: [], exchanges: [], logouts: [] };
  let index = 0;
  server.use(
    http.post(
      "*/v1/providers/:name/oauth/start",
      async ({ request, params }) => {
        recorded.starts.push({
          provider: String(params.name),
          body: await request.json(),
        });
        return HttpResponse.json(startBody);
      },
    ),
    http.post(
      "*/v1/providers/:name/oauth/exchange",
      async ({ request, params }) => {
        recorded.exchanges.push({
          provider: String(params.name),
          body: await request.json(),
        });
        const response =
          exchangeResponses[Math.min(index, exchangeResponses.length - 1)];
        index++;
        return HttpResponse.json(response);
      },
    ),
    http.post(
      "*/v1/providers/:name/oauth/logout",
      async ({ request, params }) => {
        const text = await request.text();
        recorded.logouts.push({
          provider: String(params.name),
          body: text ? (JSON.parse(text) as unknown) : null,
        });
        return HttpResponse.json({ success: true });
      },
    ),
  );
  return recorded;
}

describe("ProviderOAuth Claude Code sign-in paths", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    openSpy.mockImplementation(() => null);
  });

  test.each([
    ["bedrock", "Login with AWS SSO"],
    ["vertex", "Login with Google Cloud"],
    ["foundry", "Login with Microsoft Azure"],
  ])("labels native %s login", (backend, label) => {
    renderNative({ backend });
    expect(screen.getByText(label)).toBeInTheDocument();
  });

  test("keeps the subscription label and sends max mode", async () => {
    const recorded = mockNativeFlow([], {
      session_id: "sub",
      authorize_url: "https://claude.ai/oauth/authorize",
      mode: "callback",
    });
    const { user } = render(
      <ProviderOAuth
        providerName="claude_code"
        oauthConnected={false}
        authStatus=""
        loginKind="subscription"
        backend="oauth"
      />,
      { preloadedState: PRELOADED_STATE },
    );
    expect(screen.getByText("Login with Anthropic")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Login" }));
    await screen.findByText("Waiting for authentication...");
    expect(recorded.starts[0].body).toEqual({ mode: "max" });
  });

  test("starts Anthropic Console login in console mode", async () => {
    const recorded = mockNativeFlow([], {
      session_id: "console",
      authorize_url: "https://console.anthropic.com/oauth/authorize",
      mode: "manual_code",
    });
    const { user } = render(
      <ProviderOAuth
        providerName="claude_code"
        oauthConnected={false}
        authStatus=""
        loginKind="console"
        backend="anthropic"
      />,
      { preloadedState: PRELOADED_STATE },
    );
    expect(
      screen.getByText("Login with Anthropic Console"),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Login" }));
    await screen.findByText("Paste the authorization code");
    expect(recorded.starts[0].body).toEqual({ mode: "console" });
    expect(openSpy).toHaveBeenCalledWith(
      "https://console.anthropic.com/oauth/authorize",
      "_blank",
      "noopener,noreferrer",
    );
  });

  test.each(["bedrock", "vertex", "foundry"])(
    "polls native %s login to completion without opening an empty URL",
    async (backend) => {
      const recorded = mockNativeFlow([
        {
          success: false,
          status: "pending",
          auth_status: "Waiting for sign-in",
          output: "Open https://device.example/verify and enter WXYZ-1234",
          poll_interval: 1,
        },
        {
          success: true,
          status: "completed",
          auth_status: "CLI sign-in finished",
        },
      ]);
      const { user } = renderNative({ backend });
      await user.click(screen.getByRole("button", { name: "Login" }));

      expect(
        await screen.findByText("Finish signing in on the engine host."),
      ).toBeInTheDocument();
      expect(openSpy).not.toHaveBeenCalled();
      expect(
        await screen.findByText(
          "Open https://device.example/verify and enter WXYZ-1234",
          {},
          { timeout: 3000 },
        ),
      ).toBeInTheDocument();
      expect(
        await screen.findByText(
          /sign-in finished on the engine host/,
          {},
          { timeout: 3000 },
        ),
      ).toBeInTheDocument();
      expect(screen.queryByText(/verified/i)).toBeNull();
      expect(screen.queryByText("Connected")).toBeNull();
      expect(recorded.starts[0].body).toEqual({ mode: "max" });
      expect(recorded.exchanges[0].body).toEqual({
        session_id: "native-1",
        code: "",
      });
      expect(recorded.logouts).toHaveLength(0);
      expect(openSpy).not.toHaveBeenCalled();
    },
  );

  test("shows terminal native failure output and allows retry", async () => {
    mockNativeFlow([
      {
        success: false,
        status: "failed",
        auth_status: "aws sso login exited with an error",
        output: "Token has expired",
      },
    ]);
    const { user } = renderNative({ backend: "bedrock" });
    await user.click(screen.getByRole("button", { name: "Login" }));
    expect(
      await screen.findByText(
        "aws sso login exited with an error",
        {},
        { timeout: 3000 },
      ),
    ).toHaveAttribute("role", "alert");
    expect(screen.getByText("Token has expired")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Try again" })).toBeEnabled();
  });

  test("replaces a bare native failure status with actionable guidance", async () => {
    mockNativeFlow([
      {
        success: false,
        status: "failed",
        auth_status: "error",
      },
    ]);
    const { user } = renderNative({ backend: "bedrock" });
    await user.click(screen.getByRole("button", { name: "Login" }));
    expect(
      await screen.findByRole("alert", {}, { timeout: 3000 }),
    ).toHaveTextContent(
      "AWS SSO sign-in failed on the engine host. Check the AWS profile and SSO configuration, then try again. SSO login is not required when using existing AWS credentials or a Bedrock API key.",
    );
    expect(
      screen.queryByText("error", { exact: true }),
    ).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Try again" })).toBeEnabled();
  });

  test("surfaces a missing CLI error from start", async () => {
    server.use(
      http.post("*/v1/providers/claude_code/oauth/start", () =>
        HttpResponse.json(
          { detail: "gcloud CLI was not found on the engine host" },
          { status: 400 },
        ),
      ),
    );
    const { user } = renderNative({ backend: "vertex" });
    await user.click(screen.getByRole("button", { name: "Login" }));
    expect(
      await screen.findByText("gcloud CLI was not found on the engine host"),
    ).toBeInTheDocument();
    expect(openSpy).not.toHaveBeenCalled();
  });

  test("cancel stops polling and cancels only that session", async () => {
    const recorded = mockNativeFlow([
      {
        success: false,
        status: "pending",
        auth_status: "Waiting",
        poll_interval: 1,
      },
    ]);
    const { user } = renderNative({
      providerName: "claude_code_work",
      backend: "foundry",
    });
    await user.click(screen.getByRole("button", { name: "Login" }));
    await screen.findByText("Finish signing in on the engine host.");
    await user.click(screen.getByRole("button", { name: "Cancel" }));

    await waitFor(() => {
      expect(recorded.logouts).toEqual([
        { provider: "claude_code_work", body: { session_id: "native-1" } },
      ]);
    });
    expect(screen.getByText("Login with Microsoft Azure")).toBeInTheDocument();
    const exchangesAfterCancel = recorded.exchanges.length;
    await new Promise((resolve) => setTimeout(resolve, 1300));
    expect(recorded.exchanges.length).toBe(exchangesAfterCancel);
  });

  test("changing backend or instance cancels the stale native session", async () => {
    const recorded = mockNativeFlow([
      {
        success: false,
        status: "pending",
        auth_status: "Waiting",
        poll_interval: 1,
      },
    ]);
    const { user, rerender } = renderNative({
      providerName: "claude_a",
      backend: "bedrock",
    });
    await user.click(screen.getByRole("button", { name: "Login" }));
    await screen.findByText("Finish signing in on the engine host.");

    rerender(
      <ProviderOAuth
        providerName="claude_a"
        baseProvider="claude_code"
        oauthConnected={false}
        authStatus=""
        loginKind="native"
        backend="vertex"
      />,
    );
    expect(screen.getByText("Login with Google Cloud")).toBeInTheDocument();
    expect(
      screen.queryByText("Finish signing in on the engine host."),
    ).toBeNull();
    await waitFor(() => {
      expect(recorded.logouts).toEqual([
        { provider: "claude_a", body: { session_id: "native-1" } },
      ]);
    });

    await user.click(screen.getByRole("button", { name: "Login" }));
    await screen.findByText("Finish signing in on the engine host.");
    rerender(
      <ProviderOAuth
        providerName="claude_b"
        baseProvider="claude_code"
        oauthConnected={false}
        authStatus=""
        loginKind="native"
        backend="vertex"
      />,
    );
    await waitFor(() => {
      expect(recorded.logouts).toHaveLength(2);
    });
    expect(recorded.logouts[1].provider).toBe("claude_a");
    expect(
      recorded.exchanges.every((item) => item.provider === "claude_a"),
    ).toBe(true);
    expect(screen.getByText("Login with Google Cloud")).toBeInTheDocument();
  });

  test("readonly providers cannot start or disconnect", () => {
    renderNative({ backend: "bedrock", readonly: true });
    expect(screen.getByRole("button", { name: "Login" })).toBeDisabled();
  });

  test("readonly connected subscription cannot disconnect", () => {
    render(
      <ProviderOAuth
        providerName="claude_code"
        oauthConnected
        authStatus="Connected"
        loginKind="subscription"
        backend="oauth"
        readonly
      />,
      { preloadedState: PRELOADED_STATE },
    );
    expect(screen.getByRole("button", { name: "Disconnect" })).toBeDisabled();
  });

  test("rejects an unknown start mode", async () => {
    mockNativeFlow([], {
      session_id: "x",
      authorize_url: "",
      mode: "telepathy",
    });
    const { user } = renderNative({ backend: "bedrock" });
    await user.click(screen.getByRole("button", { name: "Login" }));
    expect(
      await screen.findByText(/Invalid response from/),
    ).toBeInTheDocument();
  });
});
