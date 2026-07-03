import { describe, expect, it, vi } from "vitest";
import { screen } from "@testing-library/react";

import { render } from "../../utils/test-utils";
import { useGetRagStatusQuery } from "../../services/refact/ragStatus";
import type { RagStatus } from "../../services/refact/types";
import { RagStatusIndicators } from "./RagStatusIndicators";

vi.mock("../../services/refact/ragStatus", () => ({
  useGetRagStatusQuery: vi.fn(),
}));

const status: RagStatus = {
  ast: null,
  ast_alive: "turned_off",
  vecdb: null,
  vecdb_alive: "turned_off",
  vec_db_error: "",
  codegraph: {
    counts: {
      nodes: 12,
      edges: 6,
      files: 3,
      fts_docs: 3,
    },
    queued: 2,
    state: "indexing",
    error: "",
  },
  codegraph_alive: "indexing",
  codegraph_error: "",
};

function mockRagStatusQuery(result: {
  data?: RagStatus;
  error?: unknown;
  isError?: boolean;
}) {
  (useGetRagStatusQuery as ReturnType<typeof vi.fn>).mockReturnValue({
    data: result.data,
    error: result.error,
    isError: result.isError ?? false,
    refetch: vi.fn(),
  });
}

function renderIndicator() {
  return render(<RagStatusIndicators />, {
    preloadedState: {
      config: {
        host: "vscode",
        lspPort: 8001,
        themeProps: {},
      },
    },
  });
}

describe("RagStatusIndicators", () => {
  it("renders only the CodeGraph chip", () => {
    mockRagStatusQuery({ data: status });

    renderIndicator();

    expect(
      screen.getByRole("button", { name: "CodeGraph status: working" }),
    ).toBeInTheDocument();
    expect(screen.queryByLabelText(/VecDB status:/i)).not.toBeInTheDocument();
    expect(screen.queryByLabelText(/AST status:/i)).not.toBeInTheDocument();
  });

  it("renders CodeGraph as stale error when the latest poll fails", () => {
    mockRagStatusQuery({
      data: status,
      error: { status: 500, data: "poll failed" },
      isError: true,
    });

    renderIndicator();

    expect(
      screen.getByRole("button", { name: "CodeGraph status: error" }),
    ).toBeInTheDocument();
  });
});
