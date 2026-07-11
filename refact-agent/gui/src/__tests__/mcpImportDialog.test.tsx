import { http, HttpResponse } from "msw";
import { describe, expect, test, vi } from "vitest";

import { MCPImportDialog } from "../components/IntegrationsView/MCPImportDialog";
import { fireEvent, render, screen, waitFor } from "../utils/test-utils";
import { server } from "../utils/mockServer";

const CONFIG_STATE = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "web" as const,
  },
};

async function openDialog() {
  const view = render(<MCPImportDialog />, { preloadedState: CONFIG_STATE });
  await view.user.click(screen.getByRole("button", { name: /^import$/i }));
  return view;
}

describe("MCPImportDialog", () => {
  test("dialog opens, invalid JSON shows error, no POST", async () => {
    const post = vi.fn();
    server.use(
      http.post("*/v1/mcp/import", async ({ request }) => {
        post(await request.json());
        return HttpResponse.json({ imported: [], skipped: [], errors: [] });
      }),
    );

    await openDialog();

    fireEvent.change(screen.getByLabelText(/mcp servers json/i), {
      target: { value: "{" },
    });
    fireEvent.click(screen.getByRole("button", { name: /^import$/i }));

    expect(screen.getByText("Invalid JSON")).toBeInTheDocument();
    expect(post).not.toHaveBeenCalled();
  });

  test("valid bare Claude Desktop JSON posts wrapped and renders imported result names", async () => {
    let postedBody: unknown;
    server.use(
      http.post("*/v1/mcp/import", async ({ request }) => {
        postedBody = await request.json();
        return HttpResponse.json({
          imported: [{ config_name: "github" }],
          skipped: [],
          errors: [],
        });
      }),
    );

    await openDialog();

    fireEvent.change(screen.getByLabelText(/mcp servers json/i), {
      target: {
        value: JSON.stringify({
          github: { command: "npx", args: ["-y", "server"] },
        }),
      },
    });
    fireEvent.click(screen.getByRole("button", { name: /^import$/i }));

    await waitFor(() => {
      expect(postedBody).toEqual({
        mcpServers: { github: { command: "npx", args: ["-y", "server"] } },
        overwrite_existing: false,
      });
    });
    expect(await screen.findByText("Imported: 1")).toBeInTheDocument();
    expect(screen.getByText("github")).toBeInTheDocument();
  });

  test("bundle JSON posts unchanged with overwrite flag", async () => {
    let postedBody: unknown;
    server.use(
      http.post("*/v1/mcp/import", async ({ request }) => {
        postedBody = await request.json();
        return HttpResponse.json({ imported: [], skipped: [], errors: [] });
      }),
    );

    const bundle = { bundle: { servers: [{ name: "linear" }] } };
    await openDialog();

    fireEvent.change(screen.getByLabelText(/mcp servers json/i), {
      target: { value: JSON.stringify(bundle) },
    });
    fireEvent.click(screen.getByRole("button", { name: /^import$/i }));

    await waitFor(() => {
      expect(postedBody).toEqual({ ...bundle, overwrite_existing: false });
    });
  });
});
