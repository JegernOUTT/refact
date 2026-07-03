import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import cytoscape from "cytoscape";
import type Cytoscape from "cytoscape";
import fcose from "cytoscape-fcose";
import CytoscapeComponent from "react-cytoscapejs";
import { GitBranch, MousePointerClick, Network, Search } from "lucide-react";

import {
  Card,
  EmptyState,
  ErrorState,
  LoadingState,
  SegmentedControl,
} from "../../components/ui";
import { useReducedMotion } from "../../hooks/useReducedMotion";
import { useGetCodeIntelGraphQuery } from "../../services/refact/codeIntel";
import type {
  CodeIntelDetail,
  CodeIntelGraph,
  CodeIntelGraphNode,
  CodeIntelResponse,
} from "../../services/refact/types";
import { useKnowledgeGraphTheme } from "../Knowledge/useKnowledgeGraphTheme";
import type { KnowledgeGraphColors } from "../Knowledge/useKnowledgeGraphTheme";
import styles from "./CodeGraphView.module.css";

cytoscape.use(fcose);

const LIMIT_OPTIONS = [100, 250, 500, 1000] as const;
const DEFAULT_LIMIT = 250;
const MAX_MAPPED_DEGREE = 20;

type LimitValue = (typeof LIMIT_OPTIONS)[number];

type CodeGraphElement = {
  data: {
    id: string;
    label?: string;
    path?: string;
    kind?: string;
    source?: string;
    target?: string;
    degree?: number;
    centrality?: number;
  };
  group: "nodes" | "edges";
};

function isCodeIntelDetail(
  response: CodeIntelResponse<CodeIntelGraph> | undefined,
): response is CodeIntelDetail {
  return typeof response === "object" && "detail" in response;
}

function normalizeKind(kind: string | undefined): string {
  const normalized = kind?.trim().toLowerCase();
  return normalized && normalized.length > 0 ? normalized : "symbol";
}

function formatKind(kind: string | undefined): string {
  return normalizeKind(kind).replace(/_/g, " ");
}

function nodeId(id: number): string {
  return String(id);
}

function buildDegreeMap(graph: CodeIntelGraph): Map<string, number> {
  const map = new Map<string, number>();

  graph.edges.forEach((edge) => {
    const source = nodeId(edge.source);
    const target = nodeId(edge.target);
    map.set(source, (map.get(source) ?? 0) + 1);
    map.set(target, (map.get(target) ?? 0) + 1);
  });

  graph.nodes.forEach((node) => {
    const id = nodeId(node.id);
    if (!map.has(id)) map.set(id, 1);
  });

  return map;
}

function buildSymbolKindColors(
  colors: KnowledgeGraphColors,
): Record<string, string> {
  const code = colors.kind.code;
  const convention = colors.kind.convention;
  const decision = colors.kind.decision;
  const domain = colors.kind.domain;

  return {
    symbol: colors.kindDefault,
    file: colors.faint,
    module: code,
    package: code,
    namespace: code,
    function: colors.accent,
    method: colors.accent,
    constructor: colors.accent,
    route_handler: domain,
    class: decision,
    struct: decision,
    enum: decision,
    trait: decision,
    interface: decision,
    type: decision,
    variable: convention,
    constant: convention,
    field: convention,
    property: convention,
  };
}

function mapGraphElements(
  graph: CodeIntelGraph,
  degreeMap: Map<string, number>,
): CodeGraphElement[] {
  const nodeIds = new Set(graph.nodes.map((node) => nodeId(node.id)));
  const nodes: CodeGraphElement[] = graph.nodes.map((node) => {
    const id = nodeId(node.id);
    const degree = degreeMap.get(id) ?? 1;

    return {
      data: {
        id,
        label: node.name,
        path: node.path,
        kind: normalizeKind(node.kind),
        degree,
        centrality: Math.min(MAX_MAPPED_DEGREE, Math.max(1, degree)),
      },
      group: "nodes",
    };
  });
  const edges: CodeGraphElement[] = graph.edges
    .filter(
      (edge) =>
        nodeIds.has(nodeId(edge.source)) && nodeIds.has(nodeId(edge.target)),
    )
    .map((edge, index) => {
      const source = nodeId(edge.source);
      const target = nodeId(edge.target);
      const kind = normalizeKind(edge.kind);

      return {
        data: {
          id: `${source}::${target}::${kind}::${index}`,
          source,
          target,
          kind,
          label: kind,
        },
        group: "edges",
      };
    });

  return [...nodes, ...edges];
}

function findLimit(value: string): LimitValue {
  return (
    LIMIT_OPTIONS.find((option) => String(option) === value) ?? DEFAULT_LIMIT
  );
}

function GraphUnavailable({ detail }: { detail: string }) {
  return (
    <Card className={styles.stateCard} padding="lg" variant="glass">
      <EmptyState
        icon={Network}
        title="CodeGraph data is not available"
        description={detail}
        variant="full"
      />
    </Card>
  );
}

function GraphEmptyState() {
  return (
    <Card className={styles.stateCard} padding="lg" variant="glass">
      <EmptyState
        icon={Search}
        title="No code graph symbols yet"
        description="Once CodeGraph indexes the workspace, symbols and relationships will appear here."
        variant="full"
      />
    </Card>
  );
}

type DetailPanelProps = {
  selectedNode: CodeIntelGraphNode | null;
  selectedDegree: number | null;
  nodeCount: number;
  edgeCount: number;
};

function DetailPanel({
  selectedDegree,
  selectedNode,
  nodeCount,
  edgeCount,
}: DetailPanelProps) {
  if (!selectedNode) {
    return (
      <Card className={styles.detailPanel} padding="md" variant="glass">
        <div className={styles.detailEmpty}>
          <MousePointerClick className={styles.detailIcon} aria-hidden="true" />
          <div>
            <h3 className={styles.detailTitle}>Select a symbol</h3>
            <p className={styles.detailText}>
              Click a graph node to inspect its name, path, kind, and connection
              count.
            </p>
          </div>
        </div>
        <div className={styles.summaryGrid} aria-label="Graph summary">
          <span>{nodeCount.toLocaleString()} nodes</span>
          <span>{edgeCount.toLocaleString()} edges</span>
        </div>
      </Card>
    );
  }

  return (
    <Card className={styles.detailPanel} padding="md" variant="glass">
      <div className={styles.detailHeader}>
        <GitBranch className={styles.detailIcon} aria-hidden="true" />
        <div className={styles.detailCopy}>
          <h3 className={styles.detailTitle}>{selectedNode.name}</h3>
          <p className={styles.detailPath}>{selectedNode.path}</p>
        </div>
      </div>
      <dl className={styles.detailList}>
        <div>
          <dt>Kind</dt>
          <dd>{formatKind(selectedNode.kind)}</dd>
        </div>
        <div>
          <dt>Degree</dt>
          <dd>{selectedDegree ?? 0}</dd>
        </div>
      </dl>
    </Card>
  );
}

export function CodeGraphView() {
  const [limit, setLimit] = useState<LimitValue>(DEFAULT_LIMIT);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const cyRef = useRef<Cytoscape.Core | null>(null);
  const layoutRef = useRef<Cytoscape.Layouts | null>(null);
  const containerRef = useRef<HTMLDivElement | null>(null);
  const [cyReady, setCyReady] = useState(false);
  const cyReadyRef = useRef(false);
  const { colors } = useKnowledgeGraphTheme();
  const reducedMotion = useReducedMotion();
  const { data, error, isFetching, isLoading } = useGetCodeIntelGraphQuery({
    limit,
  });
  const graph = isCodeIntelDetail(data) ? null : data;

  const degreeMap = useMemo(
    () => (graph ? buildDegreeMap(graph) : new Map<string, number>()),
    [graph],
  );
  const nodesById = useMemo(() => {
    const map = new Map<string, CodeIntelGraphNode>();
    graph?.nodes.forEach((node) => map.set(nodeId(node.id), node));
    return map;
  }, [graph]);
  const elements = useMemo(
    () => (graph ? mapGraphElements(graph, degreeMap) : []),
    [degreeMap, graph],
  );
  const elementSignature = useMemo(
    () => elements.map((element) => element.data.id).join("|"),
    [elements],
  );
  const selectedNode = selectedId ? nodesById.get(selectedId) ?? null : null;
  const selectedDegree = selectedId ? degreeMap.get(selectedId) ?? null : null;

  const stylesheet = useMemo<Cytoscape.StylesheetStyle[]>(() => {
    const kindColors = buildSymbolKindColors(colors);

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
          width: "mapData(centrality, 1, 20, 28, 70)",
          height: "mapData(centrality, 1, 20, 28, 70)",
          "text-wrap": "wrap",
          "text-max-width": "96px",
        },
      },
      ...Object.entries(kindColors).map(([kind, color]) => ({
        selector: `node[kind="${kind}"]`,
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
          opacity: 0.45,
        },
      },
      {
        selector: 'edge[kind="calls"]',
        style: {
          width: 1.6,
          "line-color": colors.accent,
          "target-arrow-color": colors.accent,
          opacity: 0.65,
        },
      },
      {
        selector: 'edge[kind="inherits"]',
        style: {
          width: 2,
          "line-color": colors.kind.decision,
          "target-arrow-color": colors.kind.decision,
          "target-arrow-shape": "triangle",
          opacity: 0.75,
        },
      },
      {
        selector: 'edge[kind="route_handler"]',
        style: {
          width: 2,
          "line-color": colors.kind.domain,
          "target-arrow-color": colors.kind.domain,
          "line-style": "dashed",
          opacity: 0.8,
        },
      },
      {
        selector: "node:selected",
        style: {
          "border-width": 5,
          "border-color": colors.border,
          "border-opacity": 1,
          width: "mapData(centrality, 1, 20, 38, 86)",
          height: "mapData(centrality, 1, 20, 38, 86)",
          "background-color": colors.accent,
          "z-index": 999,
        },
      },
    ];
  }, [colors]);

  const runLayout = useCallback(() => {
    if (!cyRef.current || elements.length === 0) return null;

    layoutRef.current?.stop();

    const animate = !reducedMotion && (graph?.nodes.length ?? 0) <= 180;
    const layout = cyRef.current.layout({
      name: "fcose",
      quality: "default",
      randomize: true,
      animate,
      animationDuration: animate ? 500 : 0,
      fit: true,
      padding: 48,
      nodeRepulsion: 8500,
      idealEdgeLength: 90,
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
  }, [elements.length, graph?.nodes.length, reducedMotion]);

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
        cy.fit(cy.elements(), 56);
      }
    },
    [elements.length, runLayout],
  );

  useEffect(() => {
    const cy = cyRef.current;
    if (!cy || !cyReady) return;

    let labelsVisible = cy.zoom() > 1.15;
    const updateLabels = (visible: boolean) => {
      cy.elements("node").forEach((node) => {
        node.style("label", visible ? (node.data("label") as string) : "");
      });
    };
    const handleZoom = () => {
      const shouldShow = cy.zoom() > 1.15;
      if (shouldShow === labelsVisible) return;
      labelsVisible = shouldShow;
      updateLabels(shouldShow);
    };
    const handleNodeTap = (event: Cytoscape.EventObject) => {
      setSelectedId((event.target as Cytoscape.NodeSingular).id());
    };
    const handleCanvasTap = (event: Cytoscape.EventObject) => {
      if (event.target === cy) setSelectedId(null);
    };
    const handleNodeMouseOver = (event: Cytoscape.EventObject) => {
      const node = event.target as Cytoscape.NodeSingular;
      node.style("label", node.data("label") as string);
    };
    const handleNodeMouseOut = (event: Cytoscape.EventObject) => {
      if (cy.zoom() <= 1.15) {
        (event.target as Cytoscape.NodeSingular).style("label", "");
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
  }, [cyReady]);

  useEffect(() => {
    if (!cyRef.current || !cyReady || elements.length === 0) return;

    const layout = runLayout();
    return () => {
      layout?.stop();
    };
  }, [cyReady, elementSignature, elements.length, runLayout]);

  useEffect(() => {
    if (!selectedId || nodesById.has(selectedId)) return;
    setSelectedId(null);
  }, [nodesById, selectedId]);

  useEffect(() => {
    const container = containerRef.current;
    if (!container || !cyReady) return;

    let timeoutId: number | undefined;
    const scheduleResize = () => {
      if (timeoutId !== undefined) window.clearTimeout(timeoutId);
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
    if (!cyReady || !cyRef.current) return;

    cyRef.current.elements().unselect();
    if (!selectedId) return;

    const node = cyRef.current.$id(selectedId);
    if (node.length === 0) return;

    node.select();
    if (reducedMotion) {
      cyRef.current.center(node);
      cyRef.current.zoom(1.55);
    } else {
      cyRef.current.animate({
        center: { eles: node },
        zoom: 1.55,
        duration: 500,
      });
    }
  }, [cyReady, reducedMotion, selectedId]);

  const limitControl = (
    <div className={styles.limitControl}>
      <span className={styles.limitLabel}>Top symbols</span>
      <SegmentedControl
        aria-label="Code graph limit"
        name="code-graph-limit"
        options={LIMIT_OPTIONS.map((option) => ({
          value: String(option),
          label: option.toLocaleString(),
        }))}
        size="sm"
        value={String(limit)}
        onValueChange={(value) => setLimit(findLimit(value))}
      />
    </div>
  );

  if (isLoading) {
    return (
      <div className={styles.root}>
        <div className={styles.toolbar}>{limitControl}</div>
        <LoadingState
          label="Loading code graph"
          kind="skeleton"
          variant="full"
        />
      </div>
    );
  }

  if (error) {
    return (
      <div className={styles.root}>
        <div className={styles.toolbar}>{limitControl}</div>
        <Card className={styles.stateCard} padding="lg" variant="glass">
          <ErrorState
            title="Failed to load code graph"
            description="The code intelligence graph endpoint could not be reached."
            variant="full"
          />
        </Card>
      </div>
    );
  }

  if (isCodeIntelDetail(data)) {
    return (
      <div className={styles.root}>
        <div className={styles.toolbar}>{limitControl}</div>
        <GraphUnavailable detail={data.detail} />
      </div>
    );
  }

  if (!graph || graph.nodes.length === 0) {
    return (
      <div className={styles.root}>
        <div className={styles.toolbar}>{limitControl}</div>
        <GraphEmptyState />
      </div>
    );
  }

  return (
    <div className={styles.root}>
      <div className={styles.toolbar}>
        <div className={styles.toolbarCopy}>
          <h3 className={styles.toolbarTitle}>Code graph</h3>
          <p className={styles.toolbarDescription}>
            Symbol relationships ranked by CodeGraph centrality.
          </p>
        </div>
        {limitControl}
      </div>
      {isFetching ? (
        <p className={styles.refreshing}>Refreshing graph…</p>
      ) : null}
      <Card className={styles.canvasCard} padding="none" variant="glass">
        <div
          ref={containerRef}
          className={styles.graphContainer}
          aria-label="Code graph visualization"
        >
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
      </Card>
      <DetailPanel
        edgeCount={graph.edges.length}
        nodeCount={graph.nodes.length}
        selectedDegree={selectedDegree}
        selectedNode={selectedNode}
      />
    </div>
  );
}
