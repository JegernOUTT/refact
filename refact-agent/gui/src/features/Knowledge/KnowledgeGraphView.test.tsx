import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { KnowledgeGraphView } from "./KnowledgeGraphView";
import type {
  KnowledgeGraphNode,
  KnowledgeGraphEdge,
} from "../../services/refact/types";

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
  $: ReturnType<typeof vi.fn>;
};

const reducedMotionMock = vi.hoisted(() => ({ enabled: false }));
const cytoscapeMock = vi.hoisted(() => ({
  instances: [] as MockCy[],
  elements: [] as unknown[],
}));

vi.mock("../../hooks/useReducedMotion", () => ({
  useReducedMotion: () => reducedMotionMock.enabled,
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
          if (key === "label") return "Mock Label";
          return "mock-value";
        }),
        style: vi.fn(),
        id: vi.fn(() => "mock-id"),
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
        zoom: vi.fn((args?: unknown) => (args ? undefined : 1)),
        fit: vi.fn(),
        layout: vi.fn(() => ({
          run: vi.fn(),
          stop: vi.fn(),
        })),
        elements: vi.fn(() => mockCollection),
        animate: vi.fn(),
        center: vi.fn(),
        $id: vi.fn(() => mockCollection),
        $: vi.fn(() => mockCollection),
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

const createDocNode = (
  id: string,
  type: string,
  label: string,
): KnowledgeGraphNode => ({
  id,
  node_type: type,
  label,
});

const createEdge = (
  source: string,
  target: string,
  type: string,
): KnowledgeGraphEdge => ({
  source,
  target,
  edge_type: type,
});

beforeEach(() => {
  reducedMotionMock.enabled = false;
  cytoscapeMock.instances = [];
  cytoscapeMock.elements = [];
});

afterEach(() => {
  cleanup();
});

function isEdgeElement(
  element: unknown,
): element is { group: "edges"; data: { id: string } } {
  return (
    typeof element === "object" &&
    element !== null &&
    "group" in element &&
    element.group === "edges"
  );
}

function isNodeElement(
  element: unknown,
): element is { group: "nodes"; data: { id: string; type?: string } } {
  return (
    typeof element === "object" &&
    element !== null &&
    "group" in element &&
    (element as { group?: unknown }).group === "nodes"
  );
}

describe("KnowledgeGraphView", () => {
  it("renders empty state when no nodes", () => {
    render(
      <KnowledgeGraphView
        nodes={[]}
        edges={[]}
        selectedId={null}
        onSelectId={vi.fn()}
      />,
    );

    expect(screen.getByText("No linked memories")).toBeInTheDocument();
  });

  it("renders nodes and edges correctly", () => {
    const nodes = [
      createDocNode("doc1", "doc_code", "Code Memory"),
      createDocNode("doc2", "doc_decision", "Decision Memory"),
    ];
    const edges = [createEdge("doc1", "doc2", "relates_to")];

    render(
      <KnowledgeGraphView
        nodes={nodes}
        edges={edges}
        selectedId={null}
        onSelectId={vi.fn()}
      />,
    );

    expect(screen.getByTestId("cytoscape-mock")).toBeInTheDocument();
    expect(screen.getByText("3 elements")).toBeInTheDocument();
  });

  it("filters out non-doc nodes", () => {
    const nodes = [
      createDocNode("doc1", "doc_code", "Code Memory"),
      createDocNode("tag1", "tag", "Tag Node"),
      createDocNode("file1", "file", "File Node"),
      createDocNode("doc2", "doc_decision", "Decision Memory"),
    ];
    const edges = [createEdge("doc1", "doc2", "relates_to")];

    render(
      <KnowledgeGraphView
        nodes={nodes}
        edges={edges}
        selectedId={null}
        onSelectId={vi.fn()}
      />,
    );

    expect(screen.getByText("3 elements")).toBeInTheDocument();
  });

  it("filters out edges with non-doc nodes", () => {
    const nodes = [
      createDocNode("doc1", "doc_code", "Code Memory"),
      createDocNode("tag1", "tag", "Tag Node"),
      createDocNode("doc2", "doc_decision", "Decision Memory"),
    ];
    const edges = [
      createEdge("doc1", "doc2", "relates_to"),
      createEdge("doc1", "tag1", "tagged_with"),
      createEdge("tag1", "doc2", "tagged_with"),
    ];

    render(
      <KnowledgeGraphView
        nodes={nodes}
        edges={edges}
        selectedId={null}
        onSelectId={vi.fn()}
      />,
    );

    expect(screen.getByText("3 elements")).toBeInTheDocument();
  });

  it("filters deprecated, archived, and trajectory nodes", () => {
    const nodes = [
      createDocNode("doc1", "doc_code", "Code Memory"),
      createDocNode("doc2", "doc_deprecated", "Deprecated Memory"),
      createDocNode("doc3", "doc_trajectory", "Trajectory Memory"),
      createDocNode("doc4", "doc_preference", "Preference Memory"),
      createDocNode("doc5", "doc_archived", "Archived Memory"),
    ];
    const edges = [
      createEdge("doc1", "doc2", "relates_to"),
      createEdge("doc1", "doc4", "relates_to"),
      createEdge("doc1", "doc5", "relates_to"),
    ];

    render(
      <KnowledgeGraphView
        nodes={nodes}
        edges={edges}
        selectedId={null}
        onSelectId={vi.fn()}
      />,
    );

    expect(screen.getByText("3 elements")).toBeInTheDocument();
    expect(JSON.stringify(cytoscapeMock.elements)).not.toContain("doc5");
  });

  it("uses stable unique ids for parallel fallback edges", () => {
    const nodes = [
      createDocNode("doc-1", "doc_code", "Code Memory"),
      createDocNode("doc-2", "doc_decision", "Decision Memory"),
    ];
    const edges = [
      createEdge("doc-1", "doc-2", "relates-to"),
      createEdge("doc-1", "doc-2", "relates-to"),
    ];

    render(
      <KnowledgeGraphView
        nodes={nodes}
        edges={edges}
        selectedId={null}
        onSelectId={vi.fn()}
      />,
    );

    const edgeElements = cytoscapeMock.elements.filter(isEdgeElement);

    expect(edgeElements.map((edge) => edge.data.id)).toEqual([
      "doc-1::doc-2::relates-to::0",
      "doc-1::doc-2::relates-to::1",
    ]);
  });

  it("prefers backend edge ids when available", () => {
    const nodes = [
      createDocNode("doc1", "doc_code", "Code Memory"),
      createDocNode("doc2", "doc_decision", "Decision Memory"),
    ];
    const edges = [
      { ...createEdge("doc1", "doc2", "relates_to"), id: "backend-edge-1" },
    ];

    render(
      <KnowledgeGraphView
        nodes={nodes}
        edges={edges}
        selectedId={null}
        onSelectId={vi.fn()}
      />,
    );

    expect(screen.getByTestId("cytoscape-elements")).toHaveTextContent(
      "backend-edge-1",
    );
  });

  it("removes only registered graph handlers on cleanup", () => {
    const nodes = [createDocNode("doc1", "doc_code", "Code Memory")];
    const view = render(
      <KnowledgeGraphView
        nodes={nodes}
        edges={[]}
        selectedId={null}
        onSelectId={vi.fn()}
      />,
    );
    const cy = cytoscapeMock.instances.at(-1);

    view.unmount();

    expect(cy?.off).toHaveBeenCalledWith("tap", "node", expect.any(Function));
    expect(cy?.off).toHaveBeenCalledWith("tap", expect.any(Function));
    expect(cy?.off).toHaveBeenCalledWith("zoom", expect.any(Function));
    expect(cy?.off).toHaveBeenCalledWith(
      "mouseover",
      "node",
      expect.any(Function),
    );
    expect(cy?.off).toHaveBeenCalledWith(
      "mouseout",
      "node",
      expect.any(Function),
    );
    expect(cy?.off).not.toHaveBeenCalledWith("tap");
    expect(cy?.off).not.toHaveBeenCalledWith("zoom");
    expect(cy?.off).not.toHaveBeenCalledWith("mouseover");
    expect(cy?.off).not.toHaveBeenCalledWith("mouseout");
  });

  it("disables layout and selection animation for reduced motion", () => {
    reducedMotionMock.enabled = true;
    const nodes = [createDocNode("doc1", "doc_code", "Code Memory")];

    render(
      <KnowledgeGraphView
        nodes={nodes}
        edges={[]}
        selectedId="doc1"
        onSelectId={vi.fn()}
      />,
    );
    const cy = cytoscapeMock.instances.at(-1);

    expect(cy?.layout).toHaveBeenCalledWith(
      expect.objectContaining({ animate: false, animationDuration: 0 }),
    );
    expect(cy?.center).toHaveBeenCalled();
    expect(cy?.zoom).toHaveBeenCalledWith(1.5);
    expect(cy?.animate).not.toHaveBeenCalled();
  });

  it("handles empty edges gracefully", () => {
    const nodes = [
      createDocNode("doc1", "doc_code", "Code Memory"),
      createDocNode("doc2", "doc_decision", "Decision Memory"),
    ];

    render(
      <KnowledgeGraphView
        nodes={nodes}
        edges={[]}
        selectedId={null}
        onSelectId={vi.fn()}
      />,
    );

    expect(screen.getByTestId("cytoscape-mock")).toBeInTheDocument();
    expect(screen.getByText("2 elements")).toBeInTheDocument();
  });

  it("shows loading state", () => {
    render(
      <KnowledgeGraphView
        nodes={[]}
        edges={[]}
        selectedId={null}
        onSelectId={vi.fn()}
        isLoading={true}
      />,
    );

    expect(screen.getByText("Loading graph...")).toBeInTheDocument();
  });

  it("calls onSelectId with correct ID on node click", () => {
    const onSelectId = vi.fn();
    const nodes = [createDocNode("doc1", "doc_code", "Code Memory")];

    render(
      <KnowledgeGraphView
        nodes={nodes}
        edges={[]}
        selectedId={null}
        onSelectId={onSelectId}
      />,
    );

    expect(screen.getByTestId("cytoscape-mock")).toBeInTheDocument();
  });

  it("renders all doc node types", () => {
    const nodes = [
      createDocNode("doc1", "doc_code", "Code"),
      createDocNode("doc2", "doc_decision", "Decision"),
      createDocNode("doc3", "doc_preference", "Preference"),
      createDocNode("doc4", "doc_pattern", "Pattern"),
      createDocNode("doc5", "doc_lesson", "Lesson"),
    ];
    const edges = [
      createEdge("doc1", "doc2", "relates_to"),
      createEdge("doc2", "doc3", "relates_to"),
      createEdge("doc3", "doc4", "relates_to"),
      createEdge("doc4", "doc5", "relates_to"),
    ];

    render(
      <KnowledgeGraphView
        nodes={nodes}
        edges={edges}
        selectedId={null}
        onSelectId={vi.fn()}
      />,
    );

    expect(screen.getByText("9 elements")).toBeInTheDocument();
  });

  it("renders plain 'doc' node type (without underscore)", () => {
    const nodes = [
      createDocNode("doc1", "doc", "Plain Doc Memory"),
      createDocNode("doc2", "doc_code", "Code Memory"),
    ];
    const edges = [createEdge("doc1", "doc2", "relates_to")];

    render(
      <KnowledgeGraphView
        nodes={nodes}
        edges={edges}
        selectedId={null}
        onSelectId={vi.fn()}
      />,
    );

    // Should have 2 nodes + 1 edge = 3 elements
    expect(screen.getByText("3 elements")).toBeInTheDocument();
  });

  it("derives node color kind from explicit kind, then node_type", () => {
    const nodes: KnowledgeGraphNode[] = [
      { id: "m1", node_type: "doc", label: "Memory", kind: "memory" },
      { id: "c1", node_type: "doc_code", label: "Code" },
      { id: "d1", node_type: "doc", label: "Plain Doc" },
    ];

    render(
      <KnowledgeGraphView
        nodes={nodes}
        edges={[]}
        selectedId={null}
        onSelectId={vi.fn()}
      />,
    );

    const typeById = new Map(
      cytoscapeMock.elements
        .filter(isNodeElement)
        .map((node) => [node.data.id, node.data.type]),
    );

    // Explicit frontmatter kind wins even when node_type is plain "doc".
    expect(typeById.get("m1")).toBe("memory");
    // Otherwise the "doc_" prefix is stripped to recover the kind.
    expect(typeById.get("c1")).toBe("code");
    // Plain "doc" without a kind stays "doc" (uses the default color).
    expect(typeById.get("d1")).toBe("doc");
  });
});
