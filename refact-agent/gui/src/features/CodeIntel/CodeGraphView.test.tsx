import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "../../utils/test-utils";
import { CodeGraphView } from "./CodeGraphView";
import type {
  CodeIntelGraph,
  CodeIntelResponse,
} from "../../services/refact/types";

type MockGraphResult = {
  data: CodeIntelResponse<CodeIntelGraph> | undefined;
  error: unknown;
  isFetching: boolean;
  isLoading: boolean;
};

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

const graphHookMock = vi.hoisted(() => ({
  args: [] as { limit?: number }[],
  result: {
    data: undefined,
    error: undefined,
    isFetching: false,
    isLoading: false,
  } as MockGraphResult,
}));

const cytoscapeMock = vi.hoisted(() => ({
  instances: [] as MockCy[],
  elements: [] as unknown[],
}));

vi.mock("../../hooks/useReducedMotion", () => ({
  useReducedMotion: () => false,
}));

vi.mock("../Knowledge/useKnowledgeGraphTheme", () => ({
  useKnowledgeGraphTheme: () => ({
    colors: {
      surface: "rgb(10, 10, 10)",
      panel: "rgb(20, 20, 20)",
      accent: "rgb(127, 147, 216)",
      accentSoft: "rgba(127, 147, 216, 0.4)",
      foreground: "rgb(255, 255, 255)",
      muted: "rgb(120, 120, 120)",
      faint: "rgb(80, 80, 80)",
      border: "rgb(180, 180, 180)",
      kind: {
        code: "rgb(70, 120, 220)",
        convention: "rgb(90, 180, 130)",
        decision: "rgb(210, 160, 80)",
        domain: "rgb(220, 180, 80)",
      },
      kindDefault: "rgb(120, 120, 120)",
    },
    isDark: true,
  }),
}));

vi.mock("../../services/refact/codeIntel", () => ({
  useGetCodeIntelGraphQuery: (args: { limit?: number }) => {
    graphHookMock.args.push(args);
    return graphHookMock.result;
  },
}));

vi.mock("react-cytoscapejs", () => ({
  default: ({
    cy,
    elements,
  }: {
    cy?: (cy: unknown) => void;
    elements: unknown[];
  }) => {
    cytoscapeMock.elements = elements;

    if (cy) {
      const mockNode = {
        data: vi.fn((key: string) => {
          if (key === "label") return "Mock Symbol";
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
    }

    return (
      <div data-testid="cytoscape-mock">
        <span>{elements.length} elements</span>
        <pre data-testid="cytoscape-elements">
          {JSON.stringify(elements, null, 2)}
        </pre>
      </div>
    );
  },
}));

const graphFixture: CodeIntelGraph = {
  nodes: [
    {
      id: 1,
      name: "startServer",
      path: "src/server.ts",
      kind: "function",
    },
    {
      id: 2,
      name: "Router",
      path: "src/router.ts",
      kind: "class",
    },
  ],
  edges: [{ source: 1, target: 2, kind: "calls" }],
};

function isNodeElement(element: unknown): element is {
  group: "nodes";
  data: {
    id: string;
    label: string;
    path: string;
    kind: string;
    degree: number;
  };
} {
  return (
    typeof element === "object" &&
    element !== null &&
    "group" in element &&
    (element as { group?: unknown }).group === "nodes"
  );
}

function isEdgeElement(element: unknown): element is {
  group: "edges";
  data: { source: string; target: string; kind: string };
} {
  return (
    typeof element === "object" &&
    element !== null &&
    "group" in element &&
    (element as { group?: unknown }).group === "edges"
  );
}

beforeEach(() => {
  graphHookMock.args = [];
  graphHookMock.result = {
    data: graphFixture,
    error: undefined,
    isFetching: false,
    isLoading: false,
  };
  cytoscapeMock.instances = [];
  cytoscapeMock.elements = [];
});

describe("CodeGraphView", () => {
  it("mounts cytoscape with mapped graph elements", () => {
    render(<CodeGraphView />);

    expect(screen.getByTestId("cytoscape-mock")).toBeInTheDocument();
    expect(screen.getByText("3 elements")).toBeInTheDocument();
    expect(graphHookMock.args.at(0)).toEqual({ limit: 250 });

    const nodes = cytoscapeMock.elements.filter(isNodeElement);
    const edges = cytoscapeMock.elements.filter(isEdgeElement);

    expect(nodes).toHaveLength(2);
    expect(edges).toHaveLength(1);
    expect(nodes[0]?.data).toEqual(
      expect.objectContaining({
        id: "1",
        label: "startServer",
        path: "src/server.ts",
        kind: "function",
        degree: 1,
      }),
    );
    expect(edges[0]?.data).toEqual(
      expect.objectContaining({
        source: "1",
        target: "2",
        kind: "calls",
      }),
    );
  });

  it("renders an empty state when the graph has no nodes", () => {
    graphHookMock.result = {
      data: { nodes: [], edges: [] },
      error: undefined,
      isFetching: false,
      isLoading: false,
    };

    render(<CodeGraphView />);

    expect(screen.getByText("No code graph symbols yet")).toBeInTheDocument();
    expect(screen.queryByTestId("cytoscape-mock")).not.toBeInTheDocument();
  });
});
