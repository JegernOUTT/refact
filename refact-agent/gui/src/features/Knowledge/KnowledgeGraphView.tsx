import { useEffect, useRef, useState, useMemo, useCallback } from "react";
import CytoscapeComponent from "react-cytoscapejs";
import cytoscape from "cytoscape";
import type Cytoscape from "cytoscape";
import fcose from "cytoscape-fcose";
import { Flex, Text } from "@radix-ui/themes";
import { MagnifyingGlassIcon } from "@radix-ui/react-icons";
import type {
  KnowledgeGraphNode,
  KnowledgeGraphEdge,
} from "../../services/refact/types";
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

const isDocNode = (node: KnowledgeGraphNode): boolean => {
  const nodeType = node.node_type;
  if (nodeType === "doc_deprecated" || nodeType === "doc_trajectory") {
    return false;
  }
  return nodeType === "doc" || nodeType.startsWith("doc_");
};
const NODE_COLORS: Record<string, string> = {
  code: "#3B82F6",
  decision: "#8B5CF6",
  preference: "#10B981",
  pattern: "#F59E0B",
  lesson: "#06B6D4",
  default: "#8B5CF6",
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
  const containerRef = useRef<HTMLDivElement | null>(null);
  const [cyReady, setCyReady] = useState(false);
  const cyReadyRef = useRef(false);

  const filteredNodes = useMemo(() => {
    return nodes.filter((node) => isDocNode(node));
  }, [nodes]);

  const filteredEdges = useMemo(() => {
    const nodeIds = new Set(filteredNodes.map((n) => n.id));
    return edges.filter(
      (edge) => nodeIds.has(edge.source) && nodeIds.has(edge.target),
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
          type: node.kind ?? "default",
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
      ...Object.entries(NODE_COLORS)
        .filter(([type]) => type !== "default")
        .map(([type, color]) => ({
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
          "border-width": 5,
          "border-color": "#FFFFFF",
          "border-opacity": 1,
          width: "mapData(degree, 1, 20, 40, 80)",
          height: "mapData(degree, 1, 20, 40, 80)",
          "background-color": "#A78BFA",
          "box-shadow": "0 0 20px #8B5CF6",
          "z-index": 999,
        },
      },
    ];
  }, []);

  const handleNodeClick = useCallback(
    (nodeId: string) => {
      onSelectId(nodeId);
    },
    [onSelectId],
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

    cyRef.current.on("tap", "node", (e: Cytoscape.EventObject) => {
      handleNodeClick((e.target as Cytoscape.NodeSingular).id());
    });

    cyRef.current.on("tap", (e: Cytoscape.EventObject) => {
      if (e.target === cyRef.current) {
        handleBackgroundClick();
      }
    });

    cyRef.current.on("zoom", handleZoom);

    cyRef.current.on("mouseover", "node", (e: Cytoscape.EventObject) => {
      (e.target as Cytoscape.NodeSingular).style(
        "label",
        (e.target as Cytoscape.NodeSingular).data("label") as string,
      );
    });

    cyRef.current.on("mouseout", "node", (e: Cytoscape.EventObject) => {
      const zoom = cyRef.current?.zoom() ?? 1;
      if (zoom <= 1.2) {
        (e.target as Cytoscape.NodeSingular).style("label", "");
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
    if (!cyReady || !containerRef.current || !cyRef.current) return;
    if (typeof ResizeObserver === "undefined") return;

    const ro = new ResizeObserver(() => {
      cyRef.current?.resize();
    });
    ro.observe(containerRef.current);
    return () => ro.disconnect();
  }, [cyReady]);

  useEffect(() => {
    if (!cyRef.current || !cyReady) return;

    if (layoutRef.current) {
      layoutRef.current.stop();
    }

    const layoutOpts: Cytoscape.LayoutOptions & Record<string, unknown> = {
      name: "fcose",
      animationDuration: 500,
      randomize: true,
      randomSeed: 42,
      idealEdgeLength: 220,
      nodeRepulsion: 18000,
      nodeSeparation: 90,
      edgeElasticity: 0.35,
      gravity: 0.15,
      packComponents: true,
      componentSpacing: 140,
      padding: 30,
    };

    layoutRef.current = cyRef.current.layout(layoutOpts);

    requestAnimationFrame(() => {
      cyRef.current?.resize();
      if (layoutRef.current) {
        layoutRef.current.run();
      }
    });
  }, [cyReady, elements]);

  useEffect(() => {
    if (!cyRef.current || !cyReady) return;

    cyRef.current.$("node:selected").unselect();

    if (selectedId) {
      const node = cyRef.current.$id(selectedId);
      if (node.length > 0) {
        node.select();

        const currentZoom = cyRef.current.zoom();
        const targetZoom = Math.max(currentZoom, 1.5);

        cyRef.current.animate({
          center: { eles: node },
          zoom: targetZoom,
          duration: 400,
          easing: "ease-out",
        });
      }
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
        <div className={styles.emptyStateIcon}>
          <MagnifyingGlassIcon />
        </div>
        <div className={styles.emptyStateText}>
          <p>No linked memories</p>
        </div>
      </div>
    );
  }

  return (
    <div
      ref={containerRef}
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
        stylesheet={stylesheet as Cytoscape.StylesheetStyle[]}
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
