import { describe, expect, it, beforeEach, vi } from "vitest";
import userEvent from "@testing-library/user-event";

import { push, pagesSlice } from "../Pages/pagesSlice";
import type {
  CodeIntelOverview,
  CodeIntelResponse,
} from "../../services/refact/types";
import { render, screen, within } from "../../utils/test-utils";
import { CodeIntelWorkspace } from "./CodeIntelWorkspace";

const graphViewMock = vi.hoisted(() => ({
  calls: 0,
}));

type MockOverviewResult = {
  data: CodeIntelResponse<CodeIntelOverview> | undefined;
  error: unknown;
  isFetching: boolean;
  isLoading: boolean;
};

const overviewFixture: CodeIntelOverview = {
  counts: {
    nodes: 1234,
    edges: 5678,
    files: 42,
  },
  scc_count: 3,
  largest_scc: 9,
  component_count: 6,
  top_pagerank: [{ symbol: "crate::main", score: 0.123456 }],
  top_betweenness: [{ symbol: "crate::router", score: 1.25 }],
  file_centrality: {
    top_pagerank: [{ path: "src/main.rs", score: 0.42 }],
    top_betweenness: [{ path: "src/router.rs", score: 2.5 }],
  },
  community_count: 4,
  dead_code_count: 7,
};

let mockOverviewResult: MockOverviewResult = {
  data: overviewFixture,
  error: undefined,
  isFetching: false,
  isLoading: false,
};

vi.mock("../../services/refact/codeIntel", () => ({
  useGetCodeIntelOverviewQuery: () => mockOverviewResult,
}));

vi.mock("./CodeGraphView", () => ({
  CodeGraphView: () => {
    graphViewMock.calls += 1;
    return (
      <div>
        <span>Code graph</span>
        <span>crate::main</span>
      </div>
    );
  },
}));

function renderWorkspace() {
  return render(<CodeIntelWorkspace host="web" backFromCodeIntel={vi.fn()} />);
}

describe("CodeIntelWorkspace", () => {
  beforeEach(() => {
    mockOverviewResult = {
      data: overviewFixture,
      error: undefined,
      isFetching: false,
      isLoading: false,
    };
    graphViewMock.calls = 0;
  });

  it("renders tabs and live overview KPIs", () => {
    renderWorkspace();

    expect(
      screen.getByRole("heading", { name: "Code Intelligence" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Overview" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Graph" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Health" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Risk" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Security" })).toBeInTheDocument();

    expect(screen.getByText("Nodes")).toBeInTheDocument();
    expect(screen.getByText("1,234")).toBeInTheDocument();
    expect(screen.getByText("Edges")).toBeInTheDocument();
    expect(screen.getByText("5,678")).toBeInTheDocument();
    expect(screen.getByText("Files")).toBeInTheDocument();
    expect(screen.getByText("42")).toBeInTheDocument();
    expect(screen.getByText("Communities")).toBeInTheDocument();
    expect(screen.getByText("Dead Code")).toBeInTheDocument();

    const centralitySection = screen
      .getByRole("heading", {
        name: "Centrality leaders",
      })
      .closest("section");
    if (!centralitySection) {
      throw new Error("Centrality section was not rendered");
    }
    const centrality = within(centralitySection);
    expect(centrality.getByText("crate::main")).toBeInTheDocument();
    expect(centrality.getByText("crate::router")).toBeInTheDocument();
    expect(centrality.getByText("src/main.rs")).toBeInTheDocument();
    expect(centrality.getByText("src/router.rs")).toBeInTheDocument();
  });

  it("renders graph and placeholder tabs for follow-up cards", async () => {
    const user = userEvent.setup();
    renderWorkspace();

    await user.click(screen.getByRole("tab", { name: "Graph" }));
    expect(screen.getByText("Code graph")).toBeInTheDocument();
    expect(screen.getByText("crate::main")).toBeInTheDocument();

    await user.click(screen.getByRole("tab", { name: "Security" }));
    expect(screen.getByText("Security scan coming soon")).toBeInTheDocument();
  });

  it("registers code intel pages in the pages reducer", () => {
    const state = pagesSlice.reducer(undefined, push({ name: "code intel" }));

    expect(state.at(-1)).toEqual({ name: "code intel" });
  });
});
