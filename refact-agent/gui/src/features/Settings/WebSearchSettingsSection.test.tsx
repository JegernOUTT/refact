import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "../../utils/test-utils";
import { WebSearchSettingsSection } from "./WebSearchSettingsSection";
import {
  useGetWebSearchSettingsQuery,
  useUpdateWebSearchSettingsMutation,
  useTestWebSearchProvidersMutation,
  type WebSearchSettingsResponse,
} from "../../services/refact/webSearchSettings";

vi.mock("../../services/refact/webSearchSettings", () => ({
  useGetWebSearchSettingsQuery: vi.fn(),
  useUpdateWebSearchSettingsMutation: vi.fn(),
  useTestWebSearchProvidersMutation: vi.fn(),
}));
const settings: WebSearchSettingsResponse = {
  config: {
    schema_version: 1,
    deadline_ms: 12000,
    provider_timeout_ms: 6000,
    prefer_builtin_over_native: true,
    searxng_url: "",
    providers: {
      exa: { enabled: true, api_key: "***" },
      you: { enabled: true },
      github: { enabled: true },
    },
  },
  providers: [
    {
      id: "exa",
      display_name: "Exa",
      tier: "general",
      category: null,
      key: "optional",
      key_env_vars: ["EXA_API_KEY"],
      signup_url: "https://exa.ai",
      description: "General search",
      default_enabled: true,
      credential_status: "stored",
      active: true,
    },
    {
      id: "you",
      display_name: "You.com",
      tier: "fallback",
      category: null,
      key: "optional",
      key_env_vars: ["YOU_API_KEY"],
      signup_url: null,
      description: "Fallback search",
      default_enabled: true,
      credential_status: "env",
      active: true,
    },
    {
      id: "github",
      display_name: "GitHub",
      tier: "specialist",
      category: "code",
      key: "optional",
      key_env_vars: ["GITHUB_TOKEN"],
      signup_url: null,
      description: "Code search",
      default_enabled: true,
      credential_status: "missing",
      active: true,
    },
  ],
};
const save = vi.fn(() => ({ unwrap: () => Promise.resolve(settings) }));
const testProviders = vi.fn(() => ({
  unwrap: () =>
    Promise.resolve({
      query: "test",
      reports: [
        {
          provider: "exa",
          status: "ok",
          results: 3,
          latency_ms: 42,
          detail: "Healthy",
          top: [{ title: "Example", url: "https://example.com" }],
        },
      ],
    }),
}));
beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(useGetWebSearchSettingsQuery).mockReturnValue({
    data: settings,
    isLoading: false,
    refetch: vi.fn(),
  } as unknown as ReturnType<typeof useGetWebSearchSettingsQuery>);
  vi.mocked(useUpdateWebSearchSettingsMutation).mockReturnValue([
    save,
    { isLoading: false },
  ] as unknown as ReturnType<typeof useUpdateWebSearchSettingsMutation>);
  vi.mocked(useTestWebSearchProvidersMutation).mockReturnValue([
    testProviders,
    { isLoading: false },
  ] as unknown as ReturnType<typeof useTestWebSearchProvidersMutation>);
});

describe("Web Search settings", () => {
  it("groups providers by tier with credential placeholders", () => {
    render(<WebSearchSettingsSection />);
    for (const title of ["General", "Fallback", "Specialist sources"])
      expect(screen.getByText(title)).toBeInTheDocument();
    expect(screen.getByText("code")).toBeInTheDocument();
    expect(screen.getByLabelText("Exa API key")).toHaveAttribute(
      "type",
      "password",
    );
    expect(screen.getByLabelText("Exa API key")).toHaveAttribute(
      "placeholder",
      "•••• stored",
    );
    expect(screen.getByLabelText("You.com API key")).toHaveAttribute(
      "placeholder",
      "using YOU_API_KEY from environment",
    );
  });
  it("saves only changed fields and preserves untouched stored secrets", async () => {
    const user = userEvent.setup();
    render(<WebSearchSettingsSection />);
    await user.click(screen.getByRole("switch", { name: "Enable You.com" }));
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(save).toHaveBeenCalledWith({
      config: { providers: { you: { enabled: false } } },
    });
  });
  it("sends an empty string when clearing a stored secret", async () => {
    const user = userEvent.setup();
    render(<WebSearchSettingsSection />);
    await user.click(screen.getByRole("button", { name: "Clear saved value" }));
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(save).toHaveBeenCalledWith({
      config: { providers: { exa: { api_key: "" } } },
    });
  });
  it("keeps clearing available after focusing and blurring an untouched stored secret", async () => {
    const user = userEvent.setup();
    render(<WebSearchSettingsSection />);
    await user.click(screen.getByLabelText("Exa API key"));
    await user.tab();
    expect(screen.getByRole("button", { name: "Reset" })).toBeDisabled();
    await user.click(screen.getByRole("button", { name: "Clear saved value" }));
    expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(save).toHaveBeenCalledWith({
      config: { providers: { exa: { api_key: "" } } },
    });
  });
  it("preserves a stored secret after typing and erasing a replacement", async () => {
    const user = userEvent.setup();
    render(<WebSearchSettingsSection />);
    const input = screen.getByLabelText("Exa API key");
    await user.type(input, "x");
    await user.clear(input);
    await user.click(screen.getByRole("switch", { name: "Enable You.com" }));
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(save).toHaveBeenCalledWith({
      config: { providers: { you: { enabled: false } } },
    });
  });
  it("captures a typed secret before clicking Save", async () => {
    const user = userEvent.setup();
    render(<WebSearchSettingsSection />);
    await user.type(screen.getByLabelText("Exa API key"), "new-secret");
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(save).toHaveBeenCalledWith({
      config: { providers: { exa: { api_key: "new-secret" } } },
    });
  });
  it("shows backend validation detail", async () => {
    save.mockReturnValueOnce({
      unwrap: () =>
        Promise.reject({ status: 400, data: { detail: "Invalid URL" } }),
    });
    const user = userEvent.setup();
    render(<WebSearchSettingsSection />);
    await user.click(screen.getByRole("switch", { name: "Enable You.com" }));
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Invalid URL");
  });
  it("tests individual providers and renders reports", async () => {
    const user = userEvent.setup();
    render(<WebSearchSettingsSection />);
    await user.click(screen.getByRole("button", { name: "Test Exa" }));
    expect(testProviders).toHaveBeenCalledWith({ providers: ["exa"] });
    const table = await screen.findByRole("table");
    expect(within(table).getByText("ok")).toBeInTheDocument();
    expect(within(table).getByText("42")).toBeInTheDocument();
    expect(
      within(table).getByRole("link", { name: "Example" }),
    ).toHaveAttribute("href", "https://example.com");
  });
});
