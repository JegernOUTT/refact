import { useEffect, useRef, useState, useMemo, useCallback } from "react";
import CytoscapeComponent from "react-cytoscapejs";
import cytoscape from "cytoscape";
import type Cytoscape from "cytoscape";
import fcose from "cytoscape-fcose";
import { Flex, Text } from "@radix-ui/themes";
import type { KnowledgeGraphNode, KnowledgeGraphEdge } from "../../services/refact/types";
import styles from "./KnowledgeGraphView.module.css";

cytoscape.use(fcose);

type CytoscapeElement = {
  data: {
    id: string;
    label: string;
    type?: string;
    source?: string;
    target?: string;
    degree?: number;
  };
  group?: "nodes" | "edges";
};

interface KnowledgeGraphViewProps {
  nodes: KnowledgeGraphNode[];
  edges: KnowledgeGraphEdge[];
  selectedId: string | null;
  onSelectId: (id: string | null) => void;
  isLoading?: boolean;
}

const DOC_NODE_TYPES = new Set([
  "doc_code",
  "doc_decision",
  "doc_preference",
  "doc_pattern",
  "doc_lesson",
]);

const NODE_COLORS: Record<string, string> = {
  doc_code: "#3B82F6",
  doc_decision: "#8B5CF6",
  doc_preference: "#10B981",
  doc_pattern: "#F59E0B",
  doc_lesson: "#06B6D4",
};

export function KnowledgeGraphView({
  nodes,
  edges,
  selectedId,
  onSelectId,
  isLoading = false,
}: KnowledgeGraphViewProps) {
  const cyRef = useRef<Cytoscape.Core | null>(null);
  const layoutRef = useRef<Cytoscape.Layouts | null>(null);
  const [cyReady, setCyReady] = useState(false);
  const cyReadyRef = useRef(false);

  const filteredNodes = useMemo(() => {
    return nodes.filter((node) => DOC_NODE_TYPES.has(node.node_type));
  }, [nodes]);

  const filteredEdges = useMemo(() => {
    const nodeIds = new Set(filteredNodes.map((n) => n.id));
    return edges.filter(
      (edge) => nodeIds.has(edge.source) && nodeIds.has(edge.target)
    );
  }, [filteredNodes, edges]);

  const degreeMap = useMemo(() => {
    const map = new Map<string, number>();
    filteredEdges.forEach((edge) => {
      map.set(edge.source, (map.get(edge.source) ?? 0) + 1);
      map.set(edge.target, (map.get(edge.target) ?? 0) + 1);
    });
    filteredNodes.forEach((node) => {
      if (!map.has(node.id)) map.set(node.id, 1);
    });
    return map;
  }, [filteredEdges, filteredNodes]);

  const elements: CytoscapeElement[] = useMemo(() => {
    return [
      ...filteredNodes.map((node) => ({
        data: {
          id: node.id,
          label: node.label,
          type: node.node_type,
          degree: degreeMap.get(node.id) ?? 1,
        },
        group: "nodes" as const,
      })),
      ...filteredEdges.map((edge) => ({
        data: {
          id: `${edge.source}-${edge.target}-${edge.edge_type}`,
          source: edge.source,
          target: edge.target,
          label: edge.edge_type,
        },
        group: "edges" as const,
      })),
    ];
  }, [filteredNodes, filteredEdges, degreeMap]);

  const stylesheet: unknown[] = useMemo(() => {
    return [
      {
        selector: "node",
        style: {
          "background-color": "#8B5CF6",
          label: "",
          "font-size": "12px",
          color: "#ffffff",
          "text-valign": "center",
          "text-halign": "center",
          width: "mapData(degree, 1, 20, 30, 60)",
          height: "mapData(degree, 1, 20, 30, 60)",
          "text-wrap": "wrap",
          "text-max-width": "80px",
        },
      },
      ...Object.entries(NODE_COLORS).map(([type, color]) => ({
        selector: `node[type="${type}"]`,
        style: {
          "background-color": color,
        },
      })),
      {
        selector: "edge",
        style: {
          width: 1,
          "line-color": "#9CA3AF",
          "target-arrow-color": "#9CA3AF",
          "target-arrow-shape": "triangle",
          "curve-style": "bezier",
          opacity: 0.4,
        },
      },
      {
        selector: "node:selected",
        style: {
          "border-width": 3,
          "border-color": "#8B5CF6",
          width: "mapData(degree, 1, 20, 35, 70)",
          height: "mapData(degree, 1, 20, 35, 70)",
        },
      },
    ];
  }, []);

  const handleNodeClick = useCallback(
    (nodeId: string) => {
      onSelectId(nodeId);
    },
    [onSelectId]
  );

  const handleBackgroundClick = useCallback(() => {
    onSelectId(null);
  }, [onSelectId]);

  useEffect(() => {
    if (!cyRef.current || !cyReady) return;

    const handleZoom = () => {
      if (!cyRef.current) return;
      const zoom = cyRef.current.zoom();
      cyRef.current.elements("node").forEach((node) => {
        const label = zoom > 1.2 ? (node.data("label") as string) : "";
        node.style("label", label);
      });
    };

    cyRef.current.on("tap", "node", (e) => {
      // eslint-disable-next-line @typescript-eslint/no-unsafe-call, @typescript-eslint/no-unsafe-member-access
      handleNodeClick(e.target.id() as string);
    });

    cyRef.current.on("tap", (e) => {
      if (e.target === cyRef.current) {
        handleBackgroundClick();
      }
    });

    cyRef.current.on("zoom", handleZoom);

    cyRef.current.on("mouseover", "node", (e) => {
      // eslint-disable-next-line @typescript-eslint/no-unsafe-call, @typescript-eslint/no-unsafe-member-access
      e.target.style("label", e.target.data("label") as string);
    });

    cyRef.current.on("mouseout", "node", (e) => {
      const zoom = cyRef.current?.zoom() ?? 1;
      if (zoom <= 1.2) {
        // eslint-disable-next-line @typescript-eslint/no-unsafe-call, @typescript-eslint/no-unsafe-member-access
        e.target.style("label", "");
      }
    });

    return () => {
      if (cyRef.current) {
        cyRef.current.off("tap");
        cyRef.current.off("zoom");
        cyRef.current.off("mouseover");
        cyRef.current.off("mouseout");
      }
    };
  }, [cyReady, handleNodeClick, handleBackgroundClick]);

  useEffect(() => {
    if (!cyRef.current || !cyReady) return;

    if (layoutRef.current) {
      // eslint-disable-next-line @typescript-eslint/no-unsafe-call, @typescript-eslint/no-unsafe-member-access
      layoutRef.current.stop();
    }

    const layoutOpts: Cytoscape.LayoutOptions & Record<string, unknown> = {
      name: "fcose",
      animationDuration: 500,
      randomize: false,
      idealEdgeLength: 120,
      nodeRepulsion: 5000,
      edgeElasticity: 0.5,
    };

    layoutRef.current = cyRef.current.layout(layoutOpts);

    requestAnimationFrame(() => {
      cyRef.current?.resize();
      if (layoutRef.current) {
        // eslint-disable-next-line @typescript-eslint/no-unsafe-call, @typescript-eslint/no-unsafe-member-access
        layoutRef.current.run();
      }
    });
  }, [cyReady, elements]);

  useEffect(() => {
    if (!cyRef.current || !cyReady || !selectedId) return;

    const node = cyRef.current.$id(selectedId);
    if (node.length > 0) {
      node.select();
      cyRef.current.center(node);
    }
  }, [cyReady, selectedId]);

  if (isLoading) {
    return (
      <Flex align="center" justify="center" height="100%">
        <Text>Loading graph...</Text>
      </Flex>
    );
  }

  if (filteredNodes.length === 0) {
    return (
      <div className={styles.emptyState}>
        <div className={styles.emptyStateIcon}>🔍</div>
        <div className={styles.emptyStateText}>
          <p>No linked memories</p>
        </div>
      </div>
    );
  }

  return (
    <div
      style={{
        width: "100%",
        height: "100%",
        display: "flex",
        overflow: "hidden",
      }}
    >
      <CytoscapeComponent
        elements={elements}
        style={{ width: "100%", height: "100%" }}
        // eslint-disable-next-line @typescript-eslint/no-explicit-any, @typescript-eslint/no-unsafe-assignment
        stylesheet={stylesheet as any}
        cy={(cy) => {
          cyRef.current = cy;
          if (!cyReadyRef.current) {
            cyReadyRef.current = true;
            setCyReady(true);
            cy.resize();
          }
        }}
      />
    </div>
  );
}
