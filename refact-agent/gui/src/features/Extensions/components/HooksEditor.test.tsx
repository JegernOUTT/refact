import { fireEvent, render, screen, waitFor } from "../../../utils/test-utils";
import { http, HttpResponse } from "msw";
import { describe, expect, it, vi } from "vitest";
import { server } from "../../../utils/mockServer";
import { HooksEditor } from "./HooksEditor";
import type { HookEntry } from "../../../services/refact/extensions";

const HOOKS_STATE = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "vscode" as const,
  },
};

function hookDetail(scope: "global" | "local", command: string) {
  return {
    hooks: [
      {
        event: "PreToolUse",
        command,
        matcher: scope,
        timeout: 30,
      },
    ],
    raw_yaml: `hooks:\n  - command: ${command}\n`,
    file_path:
      scope === "global"
        ? "/home/.config/refact/hooks.yaml"
        : "/project/.refact/hooks.yaml",
  };
}

describe("HooksEditor", () => {
  it("loads hooks for the initial editable scope", async () => {
    const requestedScopes: (string | null)[] = [];
    server.use(
      http.get("*/v1/ext/hooks", ({ request }) => {
        const scope = new URL(request.url).searchParams.get("scope");
        requestedScopes.push(scope);
        return HttpResponse.json(hookDetail("local", "echo local"));
      }),
    );

    render(<HooksEditor scope="local" />, { preloadedState: HOOKS_STATE });

    expect(await screen.findByDisplayValue("echo local")).toBeDefined();
    expect(requestedScopes).toEqual(["local"]);
  });

  it("refetches and resets rows when switching hook scopes", async () => {
    const requestedScopes: (string | null)[] = [];
    server.use(
      http.get("*/v1/ext/hooks", ({ request }) => {
        const scope = new URL(request.url).searchParams.get("scope");
        requestedScopes.push(scope);
        return HttpResponse.json(
          scope === "local"
            ? hookDetail("local", "echo local")
            : hookDetail("global", "echo global"),
        );
      }),
    );

    const { user } = render(<HooksEditor />, { preloadedState: HOOKS_STATE });

    expect(await screen.findByDisplayValue("echo global")).toBeDefined();

    await user.click(screen.getByRole("radio", { name: "Project" }));

    expect(await screen.findByDisplayValue("echo local")).toBeDefined();
    expect(screen.queryByDisplayValue("echo global")).toBeNull();
    expect(requestedScopes).toEqual(["global", "local"]);
  });

  it("saves the scope currently loaded in the editor", async () => {
    const saved: { scope: string | null; body: unknown }[] = [];
    server.use(
      http.get("*/v1/ext/hooks", ({ request }) => {
        const scope = new URL(request.url).searchParams.get("scope");
        return HttpResponse.json(
          scope === "local"
            ? hookDetail("local", "echo local")
            : hookDetail("global", "echo global"),
        );
      }),
      http.put("*/v1/ext/hooks", async ({ request }) => {
        saved.push({
          scope: new URL(request.url).searchParams.get("scope"),
          body: await request.json(),
        });
        return HttpResponse.json({});
      }),
    );

    const { user } = render(<HooksEditor />, { preloadedState: HOOKS_STATE });

    expect(await screen.findByDisplayValue("echo global")).toBeDefined();
    await user.click(screen.getByRole("radio", { name: "Project" }));
    expect(await screen.findByDisplayValue("echo local")).toBeDefined();

    await user.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => {
      expect(saved).toHaveLength(1);
    });
    expect(saved[0]).toEqual({
      scope: "local",
      body: { hooks: hookDetail("local", "echo local").hooks as HookEntry[] },
    });
  });

  it("blocks scope switching when unsaved edits would be discarded", async () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    const requestedScopes: (string | null)[] = [];
    server.use(
      http.get("*/v1/ext/hooks", ({ request }) => {
        const scope = new URL(request.url).searchParams.get("scope");
        requestedScopes.push(scope);
        return HttpResponse.json(hookDetail("global", "echo global"));
      }),
    );

    const { user } = render(<HooksEditor />, { preloadedState: HOOKS_STATE });

    const command = await screen.findByDisplayValue("echo global");
    fireEvent.change(command, { target: { value: "echo edited" } });

    await user.click(screen.getByRole("radio", { name: "Project" }));

    expect(confirm).toHaveBeenCalledOnce();
    expect(screen.getByRole("radio", { name: "Global" })).toBeChecked();
    expect(screen.getByDisplayValue("echo edited")).toBeDefined();
    expect(requestedScopes).toEqual(["global"]);

    confirm.mockRestore();
  });
});
