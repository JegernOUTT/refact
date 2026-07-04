import { configureStore } from "@reduxjs/toolkit";
import { describe, expect, it, beforeEach, vi } from "vitest";
import userEvent from "@testing-library/user-event";

import { push, pagesSlice, selectCurrentPage } from "../Pages/pagesSlice";
import { useAppSelector } from "../../hooks";
import type {
  BlastReport,
  CodeIntelCommunity,
  CodeIntelDeadSymbol,
  CodeIntelDuplication,
  CodeIntelGitRisk,
  CodeIntelGraph,
  CodeIntelHealth,
  CodeIntelOverview,
  CodeIntelResponse,
  SecurityFinding,
} from "../../services/refact/types";
import type {
  CodeIntelGraphQuery,
  CodeIntelListQuery,
  PrBlastRequest,
  SecurityScanRequest,
} from "../../services/refact/codeIntel";
import { render, screen, waitFor, within } from "../../utils/test-utils";
import { CodeIntelWorkspace } from "./CodeIntelWorkspace";

const mockQueryResults = vi.hoisted(() => ({
  overview: {
    data: undefined as CodeIntelResponse<CodeIntelOverview> | undefined,
    error: undefined as unknown,
    isFetching: false,
    isLoading: false,
  },
  graph: {
    data: undefined as CodeIntelResponse<CodeIntelGraph> | undefined,
    error: undefined as unknown,
    isFetching: false,
    isLoading: false,
  },
  communities: {
    data: undefined as CodeIntelResponse<CodeIntelCommunity[]> | undefined,
    error: undefined as unknown,
    isFetching: false,
    isLoading: false,
  },
  deadCode: {
    data: undefined as CodeIntelResponse<CodeIntelDeadSymbol[]> | undefined,
    error: undefined as unknown,
    isFetching: false,
    isLoading: false,
  },
  health: {
    data: undefined as CodeIntelResponse<CodeIntelHealth> | undefined,
    error: undefined as unknown,
    isFetching: false,
    isLoading: false,
  },
  gitRisk: {
    data: undefined as CodeIntelResponse<CodeIntelGitRisk> | undefined,
    error: undefined as unknown,
    isFetching: false,
    isLoading: false,
  },
  duplication: {
    data: undefined as CodeIntelResponse<CodeIntelDuplication> | undefined,
    error: undefined as unknown,
    isFetching: false,
    isLoading: false,
  },
  graphArgs: [] as CodeIntelGraphQuery[],
  healthArgs: [] as CodeIntelListQuery[],
  gitRiskArgs: [] as CodeIntelListQuery[],
  duplicationArgs: [] as CodeIntelListQuery[],
}));

const mutationMocks = vi.hoisted(() => ({
  prBlastTrigger: vi.fn(),
  securityScanTrigger: vi.fn(),
}));

type MockCy = {
  on: ReturnType<typeof vi.fn>;
  off: ReturnType<typeof vi.fn>;
  resize: ReturnType<typeof vi.fn>;
  zoom: ReturnType<typeof vi.fn>;
  fit: ReturnType<typeof vi.fn>;
  layout: ReturnType<typeof vi.fn>;
  elements: ReturnType<typeof vi.fn>;
  animate: ReturnType<typeof vi.fn>;
  center: ReturnType<typeof vi.fn>;
  $id: ReturnType<typeof vi.fn>;
};

const cytoscapeMock = vi.hoisted(() => ({
  elements: [] as unknown[],
  instances: [] as MockCy[],
}));

vi.mock("../../services/refact/codeIntel", async () => {
  const actual = await vi.importActual<
    typeof import("../../services/refact/codeIntel")
  >("../../services/refact/codeIntel");

  return {
    ...actual,
    useGetCodeIntelOverviewQuery: () => mockQueryResults.overview,
    useGetCodeIntelGraphQuery: (args: CodeIntelGraphQuery) => {
      mockQueryResults.graphArgs.push(args);
      return mockQueryResults.graph;
    },
    useGetCodeIntelCommunitiesQuery: () => mockQueryResults.communities,
    useGetCodeIntelDeadCodeQuery: () => mockQueryResults.deadCode,
    useGetCodeIntelHealthQuery: (args: CodeIntelListQuery) => {
      mockQueryResults.healthArgs.push(args);
      return mockQueryResults.health;
    },
    useGetCodeIntelGitRiskQuery: (args: CodeIntelListQuery) => {
      mockQueryResults.gitRiskArgs.push(args);
      return mockQueryResults.gitRisk;
    },
    useGetCodeIntelDuplicationQuery: (args: CodeIntelListQuery) => {
      mockQueryResults.duplicationArgs.push(args);
      return mockQueryResults.duplication;
    },
    usePrBlastMutation: () => [mutationMocks.prBlastTrigger],
    useSecurityScanMutation: () => [mutationMocks.securityScanTrigger],
  };
});

vi.mock("react-cytoscapejs", async () => {
  const React = await vi.importActual<typeof import("react")>("react");

  function CytoscapeMockComponent({
    cy,
    elements,
  }: {
    cy?: (cy: unknown) => void;
    elements: unknown[];
  }) {
    cytoscapeMock.elements = elements;

    React.useEffect(() => {
      if (!cy) return;

      const mockNode = {
        data: vi.fn((key: string) => {
          if (key === "label") return "renderApp";
          return "mock-value";
        }),
        id: vi.fn(() => "1"),
        style: vi.fn(),
      };
      const mockCollection = {
        forEach: vi.fn((callback: (node: typeof mockNode) => void) => {
          callback(mockNode);
        }),
        length: 1,
        select: vi.fn(),
        unselect: vi.fn(),
      };
      const mockCy: MockCy = {
        on: vi.fn(),
        off: vi.fn(),
        resize: vi.fn(),
        zoom: vi.fn((value?: number) => (value === undefined ? 1 : undefined)),
        fit: vi.fn(),
        layout: vi.fn(() => ({
          run: vi.fn(),
          stop: vi.fn(),
        })),
        elements: vi.fn(() => mockCollection),
        animate: vi.fn(),
        center: vi.fn(),
        $id: vi.fn(() => mockCollection),
      };

      cytoscapeMock.instances.push(mockCy);
      cy(mockCy);
    }, [cy]);

    return (
      <div data-testid="code-graph-cytoscape">{elements.length} elements</div>
    );
  }

  return { default: CytoscapeMockComponent };
});

vi.mock("echarts-for-react/lib/core", () => ({
  default: ({ className }: { className?: string }) => (
    <div className={className} data-testid="echarts-mock" />
  ),
}));

const overviewFixture: CodeIntelOverview = {
  index_state: {
    queued: 0,
    cross_file_edges: 4,
    cross_file_ready: true,
  },
  counts: {
    nodes: 1234,
    edges: 5678,
    files: 42,
  },
  scc_count: 3,
  largest_scc: 9,
  component_count: 6,
  top_pagerank: [
    { symbol: "crate::main", path: "src/main.rs", score: 0.123456 },
  ],
  top_betweenness: [
    { symbol: "crate::router", path: "src/router.rs", score: 1.25 },
  ],
  file_centrality: {
    top_pagerank: [{ path: "src/main.rs", score: 0.42 }],
    top_betweenness: [{ path: "src/router.rs", score: 2.5 }],
  },
  community_count: 4,
  dead_code_count: 7,
};

const graphFixture: CodeIntelGraph = {
  index_state: {
    queued: 0,
    cross_file_edges: 1,
    cross_file_ready: true,
  },
  nodes: [
    {
      id: 1,
      name: "renderApp",
      path: "src/app.tsx",
      kind: "function",
    },
    {
      id: 2,
      name: "createStore",
      path: "src/store.ts",
      kind: "function",
    },
  ],
  edges: [{ source: 1, target: 2, kind: "calls" }],
};

const communitiesFixture: CodeIntelCommunity[] = [
  {
    id: 1,
    label: "UI Shell",
    member_count: 14,
    cohesion: 0.82,
  },
];

const deadCodeFixture: CodeIntelDeadSymbol[] = [
  {
    name: "unusedHelper",
    path: "src/unused.ts",
    reason: "unreachable",
    confidence: 0.91,
    line: 42,
    git_recency: "last touched 400d ago; churn 1 in mined window",
    incoming_edges: 0,
    index_state: {
      queued: 0,
      dirty_paths: 0,
      pending_refs: 0,
      cross_file_edges: 1,
      cross_file_ready: true,
    },
  },
];

const healthFixture: CodeIntelHealth = {
  index_state: {
    queued: 0,
    cross_file_edges: 1,
    cross_file_ready: true,
  },
  aggregate: {
    file_count: 1,
    function_count: 3,
    avg_score: 74.5,
    grade: "B",
    max_complexity: 12,
    avg_maintainability: 80,
    avg_duplication_pct: 0.04,
    biomarker_count: 1,
    refactoring_count: 1,
  },
  files: [
    {
      path: "src/health.ts",
      lang: "TypeScript",
      score: 74.5,
      grade: "B",
      complexity: 10,
      maintainability: 80,
      max_complexity: 12,
      avg_maintainability: 80,
      function_count: 3,
      duplication_pct: 0.04,
      dry_violation: false,
      defect_score: 0,
      maintainability_score: 70,
      performance_score: 80,
      biomarker_count: 1,
      refactoring_count: 1,
      functions: [
        {
          name: "expensiveRender",
          line1: 10,
          complexity: 12,
          nesting: 2,
          loc: 40,
          maintainability: 80,
        },
      ],
      findings: [
        {
          biomarker: "long_method",
          category: "complexity",
          dimension: "Maintainability",
          severity: "Medium",
          line: 10,
          detail: "Long method",
        },
      ],
      health_impact: [
        {
          biomarker: "long_method",
          category: "complexity",
          dimension: "Maintainability",
          severity: "Medium",
          line: 10,
          detail: "Long method health impact",
          deduction: 4.5,
          capped: false,
        },
      ],
      refactorings: [
        {
          kind: "ExtractMethod",
          target: "expensiveRender",
          line: 10,
          rationale: "Split render logic",
          impact: 0.7,
          effort: "medium",
        },
      ],
    },
  ],
};

const gitRiskFixture: CodeIntelGitRisk = {
  commits_analyzed: 12,
  agent_authored_pct: 0.25,
  hotspots: [
    {
      path: "src/risky.ts",
      churn: 22,
      risk: 0.77,
      churn_risk: 0.6,
      churn_percentile: 0.9,
      temporal_score: 0.4,
      change_entropy: 0.5,
      change_entropy_pct: 0.5,
      bus_factor: 1,
      ownership_risk: true,
      knowledge_loss: false,
    },
  ],
  ownership: [
    {
      path: "src/risky.ts",
      top_owner: "ada",
      top_owner_share: 0.8,
      bus_factor: 1,
      owner_count: 2,
      ownership_risk: true,
      knowledge_loss: false,
      owners: [{ author: "ada", commits: 8, share: 0.8 }],
    },
  ],
  co_change: [{ path_a: "src/a.ts", path_b: "src/b.ts", count: 3 }],
  coupling: [{ a: "src/a.ts", b: "src/b.ts", strength: 0.7, co_changes: 3 }],
  reviewers: [{ author: "ada", score: 0.95 }],
  recent_commit_risks: [
    {
      sha: "abc123",
      summary: "Tightened risky path",
      risk: 0.8,
      top_factor_names: ["churn", "ownership"],
    },
  ],
};

const duplicationFixture: CodeIntelDuplication = {
  aggregate: {
    file_count: 4,
    clone_pair_count: 1,
    duplication_pct: 0.12,
    duplication_percent: 12,
  },
  clones: [
    {
      path_a: "src/a.ts",
      path_b: "src/b.ts",
      line_a: 10,
      line_b: 30,
      a_start_line: 10,
      a_end_line: 20,
      b_start_line: 30,
      b_end_line: 40,
      lines: 10,
      token_len: 120,
      co_change: 2,
    },
  ],
  dry_violations: [
    {
      path: "src/a.ts",
      biomarker: "duplicate_logic",
      category: "duplication",
      dimension: "Maintainability",
      severity: "Medium",
      line: 10,
      detail: "Duplicate branch logic",
    },
  ],
  test_smells: [],
};

const blastFixture: BlastReport = {
  changed_files: ["src/main.rs", "src/router.ts"],
  directly_impacted: [
    {
      path: "src/app.rs",
      symbol: "renderApp",
      distance: 1,
      via: "calls",
      kind: "behavioral",
    },
  ],
  transitively_impacted: [
    {
      path: "src/state.ts",
      symbol: "createStore",
      distance: 2,
      via: "imports",
      kind: "structural",
    },
  ],
  impacted_file_count: 2,
  risk_score: 0.62,
  suggested_reviewers: [{ author: "ada", score: 0.95 }],
};

const securityFixture: SecurityFinding[] = [
  {
    rule: "dangerous-eval",
    severity: "Critical",
    line: 12,
    snippet: "eval(userInput)",
  },
];

function resetMocks() {
  mockQueryResults.overview = {
    data: overviewFixture,
    error: undefined,
    isFetching: false,
    isLoading: false,
  };
  mockQueryResults.graph = {
    data: graphFixture,
    error: undefined,
    isFetching: false,
    isLoading: false,
  };
  mockQueryResults.communities = {
    data: communitiesFixture,
    error: undefined,
    isFetching: false,
    isLoading: false,
  };
  mockQueryResults.deadCode = {
    data: deadCodeFixture,
    error: undefined,
    isFetching: false,
    isLoading: false,
  };
  mockQueryResults.health = {
    data: healthFixture,
    error: undefined,
    isFetching: false,
    isLoading: false,
  };
  mockQueryResults.gitRisk = {
    data: gitRiskFixture,
    error: undefined,
    isFetching: false,
    isLoading: false,
  };
  mockQueryResults.duplication = {
    data: duplicationFixture,
    error: undefined,
    isFetching: false,
    isLoading: false,
  };
  mockQueryResults.graphArgs = [];
  mockQueryResults.healthArgs = [];
  mockQueryResults.gitRiskArgs = [];
  mockQueryResults.duplicationArgs = [];
  cytoscapeMock.elements = [];
  cytoscapeMock.instances = [];
  mutationMocks.prBlastTrigger.mockReset();
  mutationMocks.securityScanTrigger.mockReset();
  mutationMocks.prBlastTrigger.mockImplementation(
    (_request: PrBlastRequest) => ({
      unwrap: () => Promise.resolve(blastFixture),
    }),
  );
  mutationMocks.securityScanTrigger.mockImplementation(
    (_request: SecurityScanRequest) => ({
      unwrap: () => Promise.resolve(securityFixture),
    }),
  );
}

function CurrentPageProbe() {
  const page = useAppSelector(selectCurrentPage);
  return <output aria-label="current page">{page?.name ?? "none"}</output>;
}

function renderWorkspace() {
  return render(
    <>
      <CurrentPageProbe />
      <CodeIntelWorkspace host="web" backFromCodeIntel={vi.fn()} />
    </>,
    {
      preloadedState: { pages: [{ name: "history" }, { name: "code intel" }] },
    },
  );
}

describe("CodeIntelWorkspace", () => {
  beforeEach(() => {
    resetMocks();
  });

  it("registers code intelligence page entries in the pages reducer", () => {
    const store = configureStore({ reducer: { pages: pagesSlice.reducer } });

    store.dispatch(push({ name: "code intel" }));

    expect(store.getState().pages.at(-1)).toEqual({ name: "code intel" });
  });

  it("renders the overview tab with live CodeGraph metrics", () => {
    renderWorkspace();

    expect(
      screen.getByRole("heading", { name: "Code Intelligence" }),
    ).toBeInTheDocument();
    expect(screen.getByLabelText("current page")).toHaveTextContent(
      "code intel",
    );
    expect(screen.getByRole("tab", { name: "Overview" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Graph" })).toBeInTheDocument();
    expect(
      screen.getByRole("tab", { name: "Communities" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Dead Code" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Health" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Risk" })).toBeInTheDocument();
    expect(
      screen.getByRole("tab", { name: "Duplication" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Security" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Tools" })).toBeInTheDocument();

    expect(screen.getByText("Nodes")).toBeInTheDocument();
    expect(screen.getByText("1,234")).toBeInTheDocument();
    expect(screen.getByText("Edges")).toBeInTheDocument();
    expect(screen.getByText("5,678")).toBeInTheDocument();
    expect(screen.getByText("Centrality leaders")).toBeInTheDocument();
    expect(screen.getByText("crate::main")).toBeInTheDocument();
  });

  it("shows and hides the index readiness banner from loaded responses", () => {
    mockQueryResults.overview = {
      data: {
        ...overviewFixture,
        index_state: {
          queued: 12,
          cross_file_edges: 2,
          cross_file_ready: false,
        },
      },
      error: undefined,
      isFetching: false,
      isLoading: false,
    };

    const { rerender } = renderWorkspace();

    expect(screen.getByText("Indexing")).toBeInTheDocument();
    expect(
      screen.getByText(/Code graph is still indexing \(12 files queued\)/),
    ).toBeInTheDocument();

    mockQueryResults.overview = {
      data: overviewFixture,
      error: undefined,
      isFetching: false,
      isLoading: false,
    };
    rerender(<CodeIntelWorkspace host="web" backFromCodeIntel={vi.fn()} />);

    expect(screen.queryByText("Indexing")).not.toBeInTheDocument();
  });

  it("switches across all nine tabs without undefined CodeIntel hooks", async () => {
    const user = userEvent.setup();
    renderWorkspace();

    await user.click(screen.getByRole("tab", { name: "Graph" }));
    expect(screen.getByText("Code graph")).toBeInTheDocument();
    expect(screen.getByTestId("code-graph-cytoscape")).toHaveTextContent(
      "3 elements",
    );
    expect(mockQueryResults.graphArgs.at(0)).toEqual({ limit: 250 });

    await user.click(screen.getByRole("tab", { name: "Communities" }));
    expect(screen.getByText("Community summary")).toBeInTheDocument();
    expect(screen.getByText("Detected code communities")).toBeInTheDocument();
    expect(screen.getAllByText("UI Shell").length).toBeGreaterThan(0);

    await user.click(screen.getByRole("tab", { name: "Dead Code" }));
    expect(screen.getByText("Dead code summary")).toBeInTheDocument();
    expect(screen.getByText("Dead code candidates")).toBeInTheDocument();
    expect(screen.getAllByText("unusedHelper").length).toBeGreaterThan(0);

    await user.click(screen.getByRole("tab", { name: "Health" }));
    expect(screen.getByText("Health aggregate")).toBeInTheDocument();
    expect(screen.getByText("Worst files")).toBeInTheDocument();
    expect(screen.getAllByText("src/health.ts").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Long method health impact").length).toBeGreaterThan(0);
    expect(screen.getAllByText("-4.5 health").length).toBeGreaterThan(0);
    expect(mockQueryResults.healthArgs.at(-1)).toEqual({ limit: 25 });

    await user.click(screen.getByRole("tab", { name: "Risk" }));
    expect(screen.getByText("Git risk summary")).toBeInTheDocument();
    expect(screen.getByText("Git risk hotspots")).toBeInTheDocument();
    expect(screen.getByText("Ownership and bus factor")).toBeInTheDocument();
    expect(screen.getAllByText("src/risky.ts").length).toBeGreaterThan(0);
    expect(screen.getByText("Recent commit risks")).toBeInTheDocument();
    expect(screen.getByText("Tightened risky path")).toBeInTheDocument();
    expect(mockQueryResults.gitRiskArgs.at(-1)).toEqual({ limit: 25 });

    await user.click(screen.getByRole("tab", { name: "Duplication" }));
    expect(screen.getByText("Duplication summary")).toBeInTheDocument();
    expect(screen.getAllByText("Clone pairs").length).toBeGreaterThan(0);
    expect(
      screen.getByText("DRY violations and test smells"),
    ).toBeInTheDocument();
    expect(screen.getAllByText("src/a.ts").length).toBeGreaterThan(0);
    expect(mockQueryResults.duplicationArgs.at(-1)).toEqual({ limit: 25 });

    await user.click(screen.getByRole("tab", { name: "Security" }));
    expect(screen.getByText("Security scan coming soon")).toBeInTheDocument();

    await user.click(screen.getByRole("tab", { name: "Tools" }));
    expect(
      screen.getByRole("region", { name: "PR Blast Radius" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("region", { name: "Security Scan" }),
    ).toBeInTheDocument();

    await user.click(screen.getByRole("tab", { name: "Overview" }));
    expect(screen.getByText("Centrality leaders")).toBeInTheDocument();
  });

  it("runs Tools tab PR blast and security scan panels", async () => {
    const user = userEvent.setup();
    renderWorkspace();

    await user.click(screen.getByRole("tab", { name: "Tools" }));

    const blastPanel = screen.getByRole("region", { name: "PR Blast Radius" });
    const securityPanel = screen.getByRole("region", { name: "Security Scan" });

    expect(
      within(blastPanel).getByText("No blast run yet"),
    ).toBeInTheDocument();
    expect(
      within(securityPanel).getByText("No security scan yet"),
    ).toBeInTheDocument();

    await user.type(
      within(blastPanel).getByLabelText(/Changed files/),
      "src/main.rs{enter}src/router.ts",
    );
    await user.type(within(blastPanel).getByLabelText(/Max depth/), "2");
    await user.click(within(blastPanel).getByRole("button", { name: "Run" }));

    await waitFor(() => {
      expect(mutationMocks.prBlastTrigger).toHaveBeenCalledWith({
        changed_files: ["src/main.rs", "src/router.ts"],
        max_depth: 2,
      });
    });
    expect(
      within(blastPanel).getByText("Directly impacted"),
    ).toBeInTheDocument();
    expect(
      within(blastPanel).getByText("Transitively impacted"),
    ).toBeInTheDocument();
    expect(within(blastPanel).getByText("0.6200")).toBeInTheDocument();
    expect(within(blastPanel).getAllByText("renderApp").length).toBeGreaterThan(
      0,
    );
    expect(
      within(blastPanel).getAllByText("createStore").length,
    ).toBeGreaterThan(0);
    expect(
      within(blastPanel).getByText("Suggested reviewers"),
    ).toBeInTheDocument();
    expect(within(blastPanel).getByText(/ada/)).toBeInTheDocument();

    await user.type(
      within(securityPanel).getByLabelText(/Path/),
      "src/server.ts",
    );
    await user.click(
      within(securityPanel).getByRole("button", { name: "Scan" }),
    );

    await waitFor(() => {
      expect(mutationMocks.securityScanTrigger).toHaveBeenCalledWith({
        path: "src/server.ts",
      });
    });
    expect(
      within(securityPanel).getAllByText("dangerous-eval").length,
    ).toBeGreaterThan(0);
    expect(
      within(securityPanel).getAllByText("Critical").length,
    ).toBeGreaterThan(0);
    expect(
      within(securityPanel).getAllByText("eval(userInput)").length,
    ).toBeGreaterThan(0);
  });

  it("keeps loading, unavailable, empty, and error states consistent", async () => {
    mockQueryResults.overview = {
      data: undefined,
      error: undefined,
      isFetching: false,
      isLoading: true,
    };
    const user = userEvent.setup();
    const { rerender } = renderWorkspace();

    expect(
      screen.getByText("Loading code intelligence overview"),
    ).toBeInTheDocument();

    mockQueryResults.overview = {
      data: { detail: "CodeGraph is disabled" },
      error: undefined,
      isFetching: false,
      isLoading: false,
    };
    rerender(<CodeIntelWorkspace host="web" backFromCodeIntel={vi.fn()} />);
    expect(
      screen.getByText("CodeGraph data is not available"),
    ).toBeInTheDocument();
    expect(screen.getByText("CodeGraph is disabled")).toBeInTheDocument();

    mockQueryResults.graph = {
      data: {
        index_state: {
          queued: 0,
          cross_file_edges: 0,
          cross_file_ready: true,
        },
        nodes: [],
        edges: [],
      },
      error: undefined,
      isFetching: false,
      isLoading: false,
    };
    await user.click(screen.getByRole("tab", { name: "Graph" }));
    expect(screen.getByText("No code graph symbols yet")).toBeInTheDocument();

    mockQueryResults.communities = {
      data: undefined,
      error: new Error("boom"),
      isFetching: false,
      isLoading: false,
    };
    await user.click(screen.getByRole("tab", { name: "Communities" }));
    expect(
      screen.getByText("Failed to load code communities"),
    ).toBeInTheDocument();
  });
});
