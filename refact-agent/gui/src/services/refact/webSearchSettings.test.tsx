import { configureStore } from "@reduxjs/toolkit";
import { useState } from "react";
import { Provider } from "react-redux";
import { render, renderHook, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { WebSearchSettingsSection } from "../../features/Settings/WebSearchSettingsSection";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  webSearchSettingsApi,
  useGetWebSearchSettingsQuery,
  useUpdateWebSearchSettingsMutation,
  useTestWebSearchProvidersMutation,
} from "./webSearchSettings";

function Wrapper({ children }: { children: React.ReactNode }) {
  const [store] = useState(() =>
    configureStore({
      reducer: {
        config: () => ({ host: "ide", lspPort: 9123, apiKey: "engine-token" }),
        [webSearchSettingsApi.reducerPath]: webSearchSettingsApi.reducer,
      },
      middleware: (defaults) =>
        defaults().concat(webSearchSettingsApi.middleware),
    }),
  );
  return <Provider store={store}>{children}</Provider>;
}
afterEach(() => vi.unstubAllGlobals());
describe("Web Search settings service hooks", () => {
  it("uses the engine endpoint, auth and sparse save wrapper", async () => {
    const fetch = vi.fn((request: Request) =>
      Promise.resolve(
        new Response(
          JSON.stringify(
            request.method === "GET"
              ? { config: {}, providers: [] }
              : { config: { deadline_ms: 9000 }, providers: [] },
          ),
          { headers: { "Content-Type": "application/json" } },
        ),
      ),
    );
    vi.stubGlobal("fetch", fetch);
    const { result } = renderHook(
      () => ({
        get: useGetWebSearchSettingsQuery(undefined),
        update: useUpdateWebSearchSettingsMutation(),
      }),
      { wrapper: Wrapper },
    );
    await waitFor(() => expect(result.current.get.isSuccess).toBe(true));
    expect(fetch.mock.calls[0][0].url).toBe(
      "http://127.0.0.1:9123/v1/web-search-settings",
    );
    expect(fetch.mock.calls[0][0].headers.get("Authorization")).toBe(
      "Bearer engine-token",
    );
    await result.current.update[0]({ config: { deadline_ms: 9000 } }).unwrap();
    const post = fetch.mock.calls
      .map(([request]) => request)
      .find((request) => request.method === "POST");
    expect(await post?.text()).toEqual(
      JSON.stringify({ config: { deadline_ms: 9000 } }),
    );
  });
  it("shows saved settings and provider metadata when the follow-up GET fails", async () => {
    const initial = {
      config: {
        schema_version: 1,
        deadline_ms: 12000,
        provider_timeout_ms: 6000,
        prefer_builtin_over_native: true,
        searxng_url: "",
        providers: { brave: { enabled: true, api_key: "" } },
      },
      providers: [
        {
          id: "brave",
          display_name: "Brave",
          tier: "general",
          category: null,
          key: "required",
          key_env_vars: ["BRAVE_API_KEY"],
          signup_url: null,
          description: "General search",
          default_enabled: true,
          credential_status: "missing",
          active: false,
        },
      ],
    };
    const saved = {
      config: {
        ...initial.config,
        deadline_ms: 9000,
        providers: { brave: { enabled: true, api_key: "***" } },
      },
      providers: initial.providers.map((provider) => ({
        ...provider,
        credential_status: "stored",
        active: true,
      })),
    };
    let gets = 0;
    vi.stubGlobal(
      "fetch",
      vi.fn((request: Request) => {
        const failed = request.method === "GET" && ++gets > 1;
        return Promise.resolve(
          new Response(
            JSON.stringify(
              failed
                ? { detail: "Unavailable" }
                : request.method === "POST"
                  ? saved
                  : initial,
            ),
            {
              status: failed ? 503 : 200,
              headers: { "Content-Type": "application/json" },
            },
          ),
        );
      }),
    );
    const user = userEvent.setup();
    render(<WebSearchSettingsSection />, { wrapper: Wrapper });
    const deadline = await screen.findByLabelText("Search deadline (ms)");
    await user.clear(deadline);
    await user.type(deadline, "9000");
    await user.type(screen.getByLabelText("Brave API key"), "new-secret");
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(gets).toBe(2));
    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
      expect(screen.getByLabelText("Search deadline (ms)")).toHaveValue(9000);
      expect(screen.getByLabelText("Brave API key")).toHaveAttribute(
        "placeholder",
        "•••• stored",
      );
      expect(screen.getByText("active")).toBeInTheDocument();
      expect(
        screen.getByRole("button", { name: "Clear saved value" }),
      ).toBeInTheDocument();
    });
  });
  it("posts test query and selected providers", async () => {
    const fetch = vi.fn((_request: Request) =>
      Promise.resolve(
        new Response(JSON.stringify({ query: "rust", reports: [] }), {
          headers: { "Content-Type": "application/json" },
        }),
      ),
    );
    vi.stubGlobal("fetch", fetch);
    const { result } = renderHook(() => useTestWebSearchProvidersMutation(), {
      wrapper: Wrapper,
    });
    await result.current[0]({ query: "rust", providers: ["exa"] }).unwrap();
    const request = fetch.mock.calls[0][0];
    expect(request.url).toBe(
      "http://127.0.0.1:9123/v1/web-search-settings/test",
    );
    expect(await request.text()).toEqual(
      JSON.stringify({ query: "rust", providers: ["exa"] }),
    );
  });
});
