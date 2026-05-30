import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import { render, screen, waitFor } from "../../../utils/test-utils";
import { server } from "../../../utils/mockServer";
import { ArtifactsPanel } from "../ArtifactsPanel";

const CONFIG_STATE = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "vscode" as const,
  },
};

describe("ArtifactsPanel", () => {
  it("renders_table_with_artifacts", async () => {
    server.use(
      http.get("*/v1/buddy/artifacts", () =>
        HttpResponse.json({
          ops: [
            {
              op_id: "op-1",
              title: "Remember the shortcut",
              op_type: "create_memory",
              status: "pending",
              created_at: "2026-05-15T00:00:00Z",
            },
            {
              op_id: "op-2",
              title: "Archive stale note",
              op_type: "archive",
              status: "applied",
              created_at: "2026-05-15T01:00:00Z",
            },
          ],
        }),
      ),
    );

    render(<ArtifactsPanel />, { preloadedState: CONFIG_STATE });

    expect(await screen.findByText("📥 Memory Ops")).toBeInTheDocument();
    expect(screen.getByText("Remember the shortcut")).toBeInTheDocument();
    expect(screen.getByText("Archive stale note")).toBeInTheDocument();
    expect(screen.getByText("create_memory")).toBeInTheDocument();
    expect(screen.getByText("archive")).toBeInTheDocument();
  });

  it("approve_button_calls_mutation_with_op_id", async () => {
    let requestBody: unknown;
    server.use(
      http.get("*/v1/buddy/artifacts", () =>
        HttpResponse.json({
          ops: [
            {
              op_id: "op-approve",
              title: "Approve me",
              op_type: "create_memory",
              status: "pending",
              created_at: "2026-05-15T00:00:00Z",
            },
          ],
        }),
      ),
      http.post(
        "*/v1/buddy/artifact_approve",
        async ({ request }) => {
          requestBody = await request.json();
          return HttpResponse.text("OK");
        },
      ),
    );

    const { user } = render(<ArtifactsPanel />, {
      preloadedState: CONFIG_STATE,
    });
    await user.click(await screen.findByRole("button", { name: "Approve" }));

    await waitFor(() => {
      expect(requestBody).toEqual({ op_id: "op-approve" });
    });
  });
});
