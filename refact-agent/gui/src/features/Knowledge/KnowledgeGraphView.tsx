import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import cytoscape from "cytoscape";
import type Cytoscape from "cytoscape";
import fcose from "cytoscape-fcose";
import CytoscapeComponent from "react-cytoscapejs";
import { Search } from "lucide-react";

import { Icon, LoadingState, Surface } from "../../components/ui";
import { useReducedMotion } from "../../hooks/useReducedMotion";
import type {
  KnowledgeGraphEdge,
  KnowledgeGraphNode,
} from "../../services/refact/types";
import { isActiveKnowledgeDocNode } from "./knowledgeGraphFilters";
import styles from "./KnowledgeGraphView.module.css";
import { useKnowledgeGraphTheme } from "./useKnowledgeGraphTheme";

cytoscape.use(fcose);

type GraphEdge = KnowledgeGraphEdge & { id?: string };

// Doc nodes carry a frontmatter `kind` (e.g. "memory", "insight", "decision");
// when it is missing we derive it from `node_type` ("doc_code" -> "code",
// "doc" -> "doc"). This is what selects the node color, so it must be robust.
function resolveNodeKind(node: KnowledgeGraphNode): string {
  const raw = node.kind?.trim().toLowerCase();
  if (raw) return raw;
  const nodeType = node.node_type.toLowerCase();
  if (nodeType.startsWith("doc_")) return nodeType.slice(4);
  return "doc";
}

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
  edges: GraphEdge[];
  selectedId: string | null;
  onSelectId: (id: string | null) => void;
  isLoading?: boolean;
  isActive?: boolean;
}

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
  const reducedMotion = useReducedMotion();

  const filteredNodes = useMemo(() => {
    return nodes.filter(isActiveKnowledgeDocNode);
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
          type: resolveNodeKind(node),
          degree: degreeMap.get(node.id) ?? 1,
        },
        group: "nodes" as const,
      })),
      ...filteredEdges.map((edge, index) => ({
        data: {
          id:
            edge.id ??
            `${edge.source}::${edge.target}::${edge.edge_type}::${index}`,
          source: edge.source,
          target: edge.target,
          label: edge.edge_type,
        },
        group: "edges" as const,
      })),
    ];
  }, [filteredNodes, filteredEdges, degreeMap]);

  const stylesheet = useMemo<Cytoscape.StylesheetStyle[]>(() => {
    return [
      {
        selector: "node",
        style: {
          "background-color": colors.kindDefault,
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
      ...Object.entries(colors.kind).map(([type, color]) => ({
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

    const animate = !reducedMotion && filteredNodes.length <= 160;

    const layout = cyRef.current.layout({
      name: "fcose",
      quality: "default",
      randomize: true,
      animate,
      animationDuration: animate ? 500 : 0,
      fit: true,
      padding: 40,
      nodeRepulsion: 8000,
      idealEdgeLength: 80,
      edgeElasticity: 0.45,
      gravity: 0.35,
      gravityRange: 3.0,
      numIter: 1200,
      packComponents: true,
      nodeSeparation: 90,
      tile: false,
    } as Cytoscape.LayoutOptions);

    layoutRef.current = layout;
    layout.run();
    return layout;
  }, [elements.length, filteredNodes.length, reducedMotion]);

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
    const cy = cyRef.current;
    if (!cy || !cyReady) return;

    let labelsVisible = cy.zoom() > 1.2;
    const handleZoom = () => {
      const shouldShow = cy.zoom() > 1.2;
      if (shouldShow === labelsVisible) return;
      labelsVisible = shouldShow;
      cy.elements("node").forEach((node) => {
        node.style("label", shouldShow ? (node.data("label") as string) : "");
      });
    };

    const handleNodeTap = (e: Cytoscape.EventObject) => {
      handleNodeClick((e.target as Cytoscape.NodeSingular).id());
    };

    const handleCanvasTap = (e: Cytoscape.EventObject) => {
      if (e.target === cy) {
        handleBackgroundClick();
      }
    };

    const handleNodeMouseOver = (e: Cytoscape.EventObject) => {
      const node = e.target as Cytoscape.NodeSingular;
      node.style("label", node.data("label") as string);
    };

    const handleNodeMouseOut = (e: Cytoscape.EventObject) => {
      const zoom = cy.zoom();
      if (zoom <= 1.2) {
        (e.target as Cytoscape.NodeSingular).style("label", "");
      }
    };

    cy.on("tap", "node", handleNodeTap);
    cy.on("tap", handleCanvasTap);
    cy.on("zoom", handleZoom);
    cy.on("mouseover", "node", handleNodeMouseOver);
    cy.on("mouseout", "node", handleNodeMouseOut);

    return () => {
      cy.off("tap", "node", handleNodeTap);
      cy.off("tap", handleCanvasTap);
      cy.off("zoom", handleZoom);
      cy.off("mouseover", "node", handleNodeMouseOver);
      cy.off("mouseout", "node", handleNodeMouseOut);
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

    const timeoutId = window.setTimeout(() => resizeAndFit(false), 80);
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
        if (reducedMotion) {
          cyRef.current.center(node);
          cyRef.current.zoom(1.5);
        } else {
          cyRef.current.animate({
            center: { eles: node },
            zoom: 1.5,
            duration: 500,
          });
        }
      }
    }
  }, [cyReady, reducedMotion, selectedId]);

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
          }
        }}
      />
    </div>
  );
}
