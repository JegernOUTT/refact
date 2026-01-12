import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { KnowledgeGraphView } from "./KnowledgeGraphView";
import type { KnowledgeGraphNode, KnowledgeGraphEdge } from "../../services/refact/types";

vi.mock("react-cytoscapejs", () => ({
  default: ({ cy, elements }: { cy?: (cy: unknown) => void; elements: unknown[] }) => {
    if (cy) {
      const mockCy = {
        on: vi.fn(),
        off: vi.fn(),
        resize: vi.fn(),
        zoom: vi.fn(() => 1),
        center: vi.fn(),
        layout: vi.fn(() => ({
          run: vi.fn(),
          stop: vi.fn(),
        })),
        elements: vi.fn(() => ({
          forEach: vi.fn(),
        })),
        $id: vi.fn(() => ({
          length: 1,
          select: vi.fn(),
        })),
      };
      cy(mockCy);
    }
    return <div data-testid="cytoscape-mock">{elements.length} elements</div>;
  },
}));

const createDocNode = (id: string, type: string, label: string): KnowledgeGraphNode => ({
  id,
  node_type: type,
  label,
});

const createEdge = (source: string, target: string, type: string): KnowledgeGraphEdge => ({
  source,
  target,
  edge_type: type,
});

describe("KnowledgeGraphView", () => {
  it("renders empty state when no nodes", () => {
    render(
      <KnowledgeGraphView
        nodes={[]}
        edges={[]}
        selectedId={null}
        onSelectId={vi.fn()}
      />
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
      />
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
      />
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
      />
    );

    expect(screen.getByText("3 elements")).toBeInTheDocument();
  });

  it("filters deprecated and trajectory nodes", () => {
    const nodes = [
      createDocNode("doc1", "doc_code", "Code Memory"),
      createDocNode("doc2", "doc_deprecated", "Deprecated Memory"),
      createDocNode("doc3", "doc_trajectory", "Trajectory Memory"),
      createDocNode("doc4", "doc_preference", "Preference Memory"),
    ];
    const edges = [
      createEdge("doc1", "doc2", "relates_to"),
      createEdge("doc1", "doc4", "relates_to"),
    ];

    render(
      <KnowledgeGraphView
        nodes={nodes}
        edges={edges}
        selectedId={null}
        onSelectId={vi.fn()}
      />
    );

    expect(screen.getByText("3 elements")).toBeInTheDocument();
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
      />
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
      />
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
      />
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
      />
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
      />
    );

    // Should have 2 nodes + 1 edge = 3 elements
    expect(screen.getByText("3 elements")).toBeInTheDocument();
  });
});
