import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import { render, screen, waitFor } from "../../../utils/test-utils";
import { server } from "../../../utils/mockServer";
import { ArtifactsPanel } from "../ArtifactsPanel";
import type { Artifact, ArtifactsPage } from "../../../services/refact/buddy";

const CONFIG_STATE = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "vscode" as const,
  },
};

const ARTIFACTS: Artifact[] = [
  {
    op_id: "op-1",
    title: "Remember the shortcut",
    op_type: "create_memory",
    status: "pending",
    created_at: "2026-05-15T00:00:00Z",
    confidence: 0.91,
  },
  {
    op_id: "op-2",
    title: "Capture project preference",
    op_type: "create_memory",
    status: "pending",
    created_at: "2026-05-15T00:05:00Z",
    confidence: 0.72,
  },
  {
    op_id: "op-3",
    title: "Archive stale note",
    op_type: "archive",
    status: "applied",
    created_at: "2026-05-15T01:00:00Z",
    confidence: 0.5,
  },
];

function makeArtifactsPage(ops: Artifact[] = ARTIFACTS): ArtifactsPage {
  return {
    ops,
    total_matching: ops.length,
    pending_count: 2,
    approved_count: 0,
    applied_count: 1,
    rejected_count: 0,
    failed_count: 0,
    skipped_count: 0,
    limit: 50,
    offset: 0,
  };
}

describe("ArtifactsPanel", () => {
  it("renders_table_with_artifacts", async () => {
    server.use(
      http.get("*/v1/buddy/artifacts", () =>
        HttpResponse.json(makeArtifactsPage()),
      ),
    );

    render(<ArtifactsPanel />, { preloadedState: CONFIG_STATE });

    expect(await screen.findByText("📥 Memory Ops")).toBeInTheDocument();
    expect(screen.getByText("2 pending")).toBeInTheDocument();
    expect(screen.getByText("Remember the shortcut")).toBeInTheDocument();
    expect(screen.getByText("Capture project preference")).toBeInTheDocument();
    expect(screen.getByText("Archive stale note")).toBeInTheDocument();
    expect(screen.getAllByText("create_memory")).toHaveLength(2);
    expect(screen.getByText("archive")).toBeInTheDocument();
    expect(screen.getByText("91%")).toBeInTheDocument();
  });

  it("approve_button_calls_decision_mutation_with_op_id", async () => {
    let requestBody: unknown;
    server.use(
      http.get("*/v1/buddy/artifacts", () =>
        HttpResponse.json(makeArtifactsPage(ARTIFACTS.slice(0, 1))),
      ),
      http.post("*/v1/buddy/artifacts/decisions", async ({ request }) => {
        requestBody = await request.json();
        return HttpResponse.json({ decided: 1, failed: 0 });
      }),
    );

    const { user } = render(<ArtifactsPanel />, {
      preloadedState: CONFIG_STATE,
    });
    await user.click(await screen.findByRole("button", { name: "Approve" }));

    await waitFor(() => {
      expect(requestBody).toEqual({
        decisions: [{ op_id: "op-1", accept: true }],
      });
    });
  });

  it("approve_all_sends_visible_pending_ids", async () => {
    let requestBody: unknown;
    server.use(
      http.get("*/v1/buddy/artifacts", () =>
        HttpResponse.json(makeArtifactsPage()),
      ),
      http.post("*/v1/buddy/artifacts/decisions", async ({ request }) => {
        requestBody = await request.json();
        return HttpResponse.json({ decided: 2, failed: 0 });
      }),
    );

    const { user } = render(<ArtifactsPanel />, {
      preloadedState: CONFIG_STATE,
    });
    await user.click(
      await screen.findByRole("button", { name: "Approve all" }),
    );

    await waitFor(() => {
      expect(requestBody).toEqual({
        decisions: [
          { op_id: "op-1", accept: true },
          { op_id: "op-2", accept: true },
        ],
      });
    });
  });
});
