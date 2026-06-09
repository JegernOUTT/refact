import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import cytoscape from "cytoscape";
import type Cytoscape from "cytoscape";
import fcose from "cytoscape-fcose";
import CytoscapeComponent from "react-cytoscapejs";
import { Search } from "lucide-react";

import { Icon, LoadingState, Surface } from "../../components/ui";
import type {
  KnowledgeGraphEdge,
  KnowledgeGraphNode,
} from "../../services/refact/types";
import styles from "./KnowledgeGraphView.module.css";
import { useKnowledgeGraphTheme } from "./useKnowledgeGraphTheme";

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
  isActive?: boolean;
}

const isDocNode = (node: KnowledgeGraphNode): boolean => {
  const nodeType = node.node_type;
  if (nodeType === "doc_deprecated" || nodeType === "doc_trajectory") {
    return false;
  }
  return nodeType === "doc" || nodeType.startsWith("doc_");
};

export function KnowledgeGraphView({
  nodes,
  edges,
  selectedId,
  onSelectId,
  isLoading = false,
  isActive = true,
}: KnowledgeGraphViewProps) {
  const cyRef = useRef<Cytoscape.Core | null>(null);
  const layoutRef = useRef<Cytoscape.Layouts | null>(null);
  const containerRef = useRef<HTMLDivElement | null>(null);
  const [cyReady, setCyReady] = useState(false);
  const cyReadyRef = useRef(false);
  const { colors } = useKnowledgeGraphTheme();

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

  const stylesheet = useMemo<Cytoscape.StylesheetStyle[]>(() => {
    const nodeColors: Record<string, string> = {
      code: colors.kind.code,
      decision: colors.kind.decision,
      preference: colors.kind.preference,
      pattern: colors.kind.pattern,
      lesson: colors.kind.lesson,
    };

    return [
      {
        selector: "node",
        style: {
          "background-color": colors.kind.other,
          label: "",
          "font-size": "12px",
          color: colors.foreground,
          "text-valign": "center",
          "text-halign": "center",
          width: "mapData(degree, 1, 20, 30, 60)",
          height: "mapData(degree, 1, 20, 30, 60)",
          "text-wrap": "wrap",
          "text-max-width": "80px",
        },
      },
      ...Object.entries(nodeColors).map(([type, color]) => ({
        selector: `node[type="${type}"]`,
        style: {
          "background-color": color,
        },
      })),
      {
        selector: "edge",
        style: {
          width: 1,
          "line-color": colors.muted,
          "target-arrow-color": colors.muted,
          "target-arrow-shape": "triangle",
          "curve-style": "bezier",
          opacity: 0.5,
        },
      },
      {
        selector: "node:selected",
        style: {
          "border-width": 5,
          "border-color": colors.border,
          "border-opacity": 1,
          width: "mapData(degree, 1, 20, 40, 80)",
          height: "mapData(degree, 1, 20, 40, 80)",
          "background-color": colors.accent,
          "box-shadow": `0 0 20px ${colors.accentSoft}`,
          "z-index": 999,
        },
      },
    ];
  }, [colors]);

  const runLayout = useCallback(() => {
    if (!cyRef.current || elements.length === 0) return null;

    if (layoutRef.current) {
      layoutRef.current.stop();
    }

    const layout = cyRef.current.layout({
      name: "fcose",
      quality: "default",
      randomize: false,
      animate: true,
      animationDuration: 500,
      fit: true,
      padding: 50,
      nodeRepulsion: 4500,
      idealEdgeLength: 100,
      edgeElasticity: 0.45,
      nestingFactor: 0.1,
      gravity: 0.25,
      numIter: 2500,
      tile: true,
      tilingPaddingVertical: 10,
      tilingPaddingHorizontal: 10,
    } as Cytoscape.LayoutOptions);

    layoutRef.current = layout;
    layout.run();
    return layout;
  }, [elements]);

  const resizeAndFit = useCallback(
    (rerunLayout = false) => {
      const cy = cyRef.current;
      const container = containerRef.current;
      if (!cy || !container) return;

      const { width, height } = container.getBoundingClientRect();
      if (width <= 0 || height <= 0) return;

      cy.resize();
      if (rerunLayout) {
        runLayout();
      } else if (elements.length > 0) {
        cy.fit(cy.elements(), 50);
      }
    },
    [elements.length, runLayout],
  );

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
    if (!cyRef.current || !cyReady || elements.length === 0) return;

    const layout = runLayout();

    return () => {
      layout?.stop();
    };
  }, [cyReady, elements.length, runLayout]);

  useEffect(() => {
    if (!cyReady || !isActive) return;

    const timeoutId = window.setTimeout(() => resizeAndFit(true), 80);
    return () => window.clearTimeout(timeoutId);
  }, [cyReady, isActive, resizeAndFit]);

  useEffect(() => {
    const container = containerRef.current;
    if (!container || !cyReady) return;

    let timeoutId: number | undefined;
    const scheduleResize = () => {
      if (timeoutId !== undefined) {
        window.clearTimeout(timeoutId);
      }
      timeoutId = window.setTimeout(() => resizeAndFit(), 80);
    };

    if (typeof ResizeObserver === "undefined") {
      scheduleResize();
      return () => {
        if (timeoutId !== undefined) window.clearTimeout(timeoutId);
      };
    }

    const observer = new ResizeObserver(scheduleResize);
    observer.observe(container);
    scheduleResize();

    return () => {
      observer.disconnect();
      if (timeoutId !== undefined) window.clearTimeout(timeoutId);
    };
  }, [cyReady, resizeAndFit]);

  useEffect(() => {
    if (!cyRef.current || !cyReady) return;

    cyRef.current.elements().unselect();
    if (selectedId) {
      const node = cyRef.current.$id(selectedId);
      if (node.length > 0) {
        node.select();
        cyRef.current.animate({
          center: { eles: node },
          zoom: 1.5,
          duration: 500,
        });
      }
    }
  }, [cyReady, selectedId]);

  if (isLoading) {
    return (
      <LoadingState
        className={`${styles.loadingState} rf-enter`}
        label="Loading graph..."
      />
    );
  }

  if (filteredNodes.length === 0) {
    return (
      <Surface
        className={styles.emptyState}
        radius="none"
        variant="plain"
        animated
      >
        <Icon icon={Search} size="lg" tone="faint" />
        <p className={styles.emptyStateText}>No linked memories</p>
      </Surface>
    );
  }

  return (
    <div ref={containerRef} className={`${styles.graphContainer} rf-enter`}>
      <CytoscapeComponent
        className={styles.graphCanvas}
        elements={elements}
        stylesheet={stylesheet}
        cy={(cy) => {
          cyRef.current = cy;
          if (!cyReadyRef.current) {
            cyReadyRef.current = true;
            setCyReady(true);
            cy.resize();
            window.setTimeout(() => resizeAndFit(true), 0);
          }
        }}
      />
    </div>
  );
}
