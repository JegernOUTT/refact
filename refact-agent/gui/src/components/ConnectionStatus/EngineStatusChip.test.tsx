import { afterEach, describe, expect, it, vi } from "vitest";
import { http, HttpResponse } from "msw";

import { render, screen } from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import type { RagStatus } from "../../services/refact/types";
import { EngineStatusChip } from "./EngineStatusChip";

const ragStatus: RagStatus = {
  ast: null,
  ast_alive: "turned_off",
  vecdb: null,
  vecdb_alive: "turned_off",
  vec_db_error: "",
  codegraph: {
    counts: { nodes: 12, edges: 6, files: 3, fts_docs: 3 },
    queued: 0,
    state: "working",
    error: "",
  },
  codegraph_alive: "working",
  codegraph_error: "",
};

const config = {
  host: "web" as const,
  lspPort: 8001,
  lspUrl: "https://engine.example.com/refact/v1/ping/Refact",
  themeProps: {},
};

function useRagHandler() {
  server.use(http.get("*/v1/rag-status", () => HttpResponse.json(ragStatus)));
}

function renderChip() {
  return render(<EngineStatusChip />, { preloadedState: { config } });
}

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("EngineStatusChip", () => {
  it("renders a pill trigger with the engine host label", () => {
    window.__REFACT_ENGINE_ORIGIN_CANDIDATES__ = [];
    useRagHandler();
    renderChip();

    const chip = screen.getByRole("button", { name: "Engine status" });
    expect(chip).toBeInTheDocument();
    expect(chip).toHaveTextContent("engine.example.com/refact");
  });

  it("opens a popover with connection details, the engine URL and the daemon action", async () => {
    window.__REFACT_ENGINE_ORIGIN_CANDIDATES__ = [];
    useRagHandler();
    const view = renderChip();

    await view.user.click(
      screen.getByRole("button", { name: "Engine status" }),
    );

    expect(
      await screen.findByLabelText(
        "Engine URL https://engine.example.com/refact",
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Refact Daemon" }),
    ).toBeInTheDocument();
    expect(
      await screen.findByRole("button", { name: /CodeGraph status:/ }),
    ).toBeInTheDocument();
  });

  it("opens the daemon page from the popover action", async () => {
    window.__REFACT_ENGINE_ORIGIN_CANDIDATES__ = [];
    useRagHandler();
    const view = renderChip();

    await view.user.click(
      screen.getByRole("button", { name: "Engine status" }),
    );
    await view.user.click(
      await screen.findByRole("button", { name: "Refact Daemon" }),
    );

    expect(view.store.getState().pages.at(-1)).toEqual({
      name: "refact daemon",
    });
  });

  it("opens the engine URL externally instead of navigating", async () => {
    window.__REFACT_ENGINE_ORIGIN_CANDIDATES__ = [];
    useRagHandler();
    const open = vi.spyOn(window, "open").mockReturnValue(null);
    const view = renderChip();

    await view.user.click(
      screen.getByRole("button", { name: "Engine status" }),
    );
    await view.user.click(
      await screen.findByLabelText(
        "Engine URL https://engine.example.com/refact",
      ),
    );

    expect(open).toHaveBeenCalledWith(
      "https://engine.example.com/refact",
      "_blank",
      "noopener,noreferrer",
    );
  });
});
