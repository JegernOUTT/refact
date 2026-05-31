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

  test("renders Claude Code manual-code flow after OAuth start", async () => {
    mockProviderOauthStart("claude_code", {
      session_id: "claude-session",
      authorize_url: "https://claude.ai/oauth/authorize",
    });

    const { user } = renderProviderOAuth("claude_code");

    await user.click(screen.getByRole("button", { name: "Login" }));

    await waitFor(() => {
      expect(
        screen.getByText("Paste the authorization code"),
      ).toBeInTheDocument();
    });
    expect(
      screen.getByPlaceholderText("Paste code here..."),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Connect" })).toBeDisabled();
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
