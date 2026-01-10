import { useEffect, useRef, useState, useMemo, useCallback } from "react";
import CytoscapeComponent from "react-cytoscapejs";
import cytoscape from "cytoscape";
import type Cytoscape from "cytoscape";
import fcose from "cytoscape-fcose";
import { Flex, Text, Checkbox } from "@radix-ui/themes";
import { useGetKnowledgeGraphQuery } from "../../services/refact/knowledgeGraphApi";
import { useKnowledgeGraphTheme } from "./useKnowledgeGraphTheme";
import { buildSubgraph } from "./knowledgeGraphSubgraph";
import type { KnowledgeGraphNode } from "../../services/refact/types";
import styles from "./KnowledgeGraph.module.css";

// Register fcose layout extension
cytoscape.use(fcose);

type FilterState = {
  kinds: Set<string>;
  statuses: Set<string>;
  tags: Set<string>;
};

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

type ViewMode = "overview" | "focus";

type VisibleNodeGroups = {
  docs: boolean;
  tags: boolean;
  files: boolean;
  entities: boolean;
};

export function KnowledgeGraph() {
  const { data: graph, isLoading, error } = useGetKnowledgeGraphQuery(undefined);
  const { colors } = useKnowledgeGraphTheme();
  const cyRef = useRef<Cytoscape.Core | null>(null);
  const layoutRef = useRef<any>(null);
  const [selectedNode, setSelectedNode] = useState<string | null>(null);
  const [mode, setMode] = useState<ViewMode>("overview");
  const [focusSeedId, setFocusSeedId] = useState<string | null>(null);
  const [focusDepth, setFocusDepth] = useState<1 | 2>(1);
  const [visibleNodeGroups, setVisibleNodeGroups] = useState<VisibleNodeGroups>({
    docs: true,
    tags: true,
    files: true,
    entities: false,
  });
  const [filters, setFilters] = useState<FilterState>({
    kinds: new Set(["code", "decision", "trajectory", "preference"]),
    statuses: new Set(["active", "deprecated"]),
    tags: new Set<string>(),
  });

  const handleNodeClick = useCallback((nodeId: string) => {
    setSelectedNode(nodeId);
    if (mode === "overview" && graph) {
      const node = graph.nodes.find((n) => n.id === nodeId);
      if (node && node.node_type.startsWith("doc_")) {
        setFocusSeedId(nodeId);
        setMode("focus");
      }
    }
  }, [mode, graph]);

  const handleBackToOverview = useCallback(() => {
    setMode("overview");
    setFocusSeedId(null);
    setSelectedNode(null);
  }, []);

  const includeNodeByType = useCallback((node: KnowledgeGraphNode): boolean => {
    const nodeType = node.node_type.toLowerCase();
    
    if (nodeType.startsWith("doc_")) {
      const kind = nodeType.replace("doc_", "");
      return filters.kinds.has(kind);
    }
    
    if (nodeType === "tag") return visibleNodeGroups.tags;
    if (nodeType === "file") return visibleNodeGroups.files;
    if (nodeType === "entity") return visibleNodeGroups.entities;
    
    return true;
  }, [filters.kinds, visibleNodeGroups]);

  const { filteredNodes, filteredEdges } = useMemo(() => {
    if (!graph) return { filteredNodes: [], filteredEdges: [] };

    if (mode === "overview") {
      const nodes = graph.nodes.filter((node) => {
        const nodeType = node.node_type.toLowerCase();
        if (nodeType.startsWith("doc_")) {
          const kind = nodeType.replace("doc_", "");
          return filters.kinds.has(kind);
        }
        return false;
      });

      const nodeIds = new Set(nodes.map((n) => n.id));
      const edges = graph.edges.filter(
        (edge) => nodeIds.has(edge.source) && nodeIds.has(edge.target),
      );

      console.log(`[KnowledgeGraph] Filtered nodes: ${nodes.length} / ${graph.nodes.length}, Edges: ${edges.length} / ${graph.edges.length}`, {
        filterKinds: Array.from(filters.kinds),
        nodeTypeSample: graph.nodes.slice(0, 5).map(n => n.node_type),
      });

      return { filteredNodes: nodes, filteredEdges: edges };
    }

    if (mode === "focus" && focusSeedId) {
      const subgraph = buildSubgraph({
        seedId: focusSeedId,
        depth: focusDepth,
        nodes: graph.nodes,
        edges: graph.edges,
        includeNode: includeNodeByType,
      });

      const nodes = graph.nodes.filter((node) => subgraph.nodeIds.has(node.id));
      const edges = graph.edges.filter((edge) => {
        const edgeId = `${edge.source}|${edge.target}|${edge.edge_type}`;
        return subgraph.edgeIds.has(edgeId);
      });

      return { filteredNodes: nodes, filteredEdges: edges };
    }

    return { filteredNodes: [], filteredEdges: [] };
  }, [mode, focusSeedId, focusDepth, graph, filters.kinds, includeNodeByType]);

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

  const elements: CytoscapeElement[] = [
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
        id: `${edge.source}|${edge.target}|${edge.edge_type}`,
        source: edge.source,
        target: edge.target,
        label: edge.edge_type,
      },
      group: "edges" as const,
    })),
  ];

  const stylesheet: any[] = [
    {
      selector: "node",
      style: {
        "background-color": colors.accent,
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
    {
      selector: 'node[type="doc_code"]',
      style: {
        "background-color": colors.kind.code,
      },
    },
    {
      selector: 'node[type="doc_decision"]',
      style: {
        "background-color": colors.kind.decision,
      },
    },
    {
      selector: 'node[type="doc_trajectory"]',
      style: {
        "background-color": colors.kind.trajectory,
      },
    },
    {
      selector: 'node[type="doc_preference"]',
      style: {
        "background-color": colors.kind.preference,
      },
    },
    {
      selector: 'node[type="tag"]',
      style: {
        "background-color": colors.kind.other,
      },
    },
    {
      selector: 'node[type="file"]',
      style: {
        "background-color": colors.kind.other,
      },
    },
    {
      selector: 'node[type="entity"]',
      style: {
        "background-color": colors.kind.other,
      },
    },
    {
      selector: "edge",
      style: {
        width: 1,
        "line-color": colors.gray,
        "target-arrow-color": colors.gray,
        "target-arrow-shape": "triangle",
        "curve-style": "bezier",
        opacity: 0.6,
      },
    },
    {
      selector: "node:selected",
      style: {
        "border-width": 3,
        "border-color": colors.accent,
        "background-color": colors.accent,
      },
    },
  ];

  useEffect(() => {
    if (cyRef.current) {
      cyRef.current.on("tap", "node", (e: any) => {
        const nodeId = e.target.id();
        handleNodeClick(nodeId);
      });

      cyRef.current.on("tap", (e: any) => {
        if (e.target === cyRef.current) {
          setSelectedNode(null);
        }
      });

      const handleZoom = () => {
        if (!cyRef.current) return;
        const zoom = cyRef.current.zoom();
        cyRef.current.elements("node").forEach((node: any) => {
          node.style("label", zoom > 1.2 ? node.data("label") : "");
        });
      };

      cyRef.current.on("zoom", handleZoom);

      cyRef.current.on("mouseover", "node", (e: any) => {
        e.target.style("label", e.target.data("label"));
      });

      cyRef.current.on("mouseout", "node", (e: any) => {
        const zoom = cyRef.current?.zoom() || 1;
        if (zoom <= 1.2) {
          e.target.style("label", "");
        }
      });
    }
  }, [handleNodeClick]);

  useEffect(() => {
    if (!cyRef.current) return;

    layoutRef.current?.stop();

    const layoutOpts = mode === "overview"
      ? {
          name: "concentric",
          animationDuration: 500,
          randomize: false,
          minNodeSpacing: 10,
          concentric: (node: any) => node.degree(),
          levelWidth: () => 1,
          startAngle: 0,
          sweep: undefined,
          clockwise: true,
          equidistant: false,
        }
      : {
          name: "fcose",
          animationDuration: 500,
          randomize: false,
          idealEdgeLength: 100,
          nodeRepulsion: 4500,
          edgeElasticity: 0.45,
        };

    layoutRef.current = cyRef.current.layout(layoutOpts as any);
    layoutRef.current.run();
  }, [mode, focusSeedId, elements]);

  const handleKindToggle = (kind: string) => {
    setFilters((prev) => {
      const newKinds = new Set(prev.kinds);
      if (newKinds.has(kind)) {
        newKinds.delete(kind);
      } else {
        newKinds.add(kind);
      }
      return { ...prev, kinds: newKinds };
    });
  };

  const handleNodeGroupToggle = (group: keyof VisibleNodeGroups) => {
    setVisibleNodeGroups((prev) => ({
      ...prev,
      [group]: !prev[group],
    }));
  };

  const selectedNodeData = selectedNode && graph
    ? graph.nodes.find((n) => n.id === selectedNode)
    : null;

  const filteredNodeIds = useMemo(() => {
    return new Set(filteredNodes.map((n) => n.id));
  }, [filteredNodes]);

  useEffect(() => {
    if (selectedNode && !filteredNodeIds.has(selectedNode)) {
      setSelectedNode(null);
    }
  }, [selectedNode, filteredNodeIds]);

  const resetFilters = useCallback(() => {
    setFilters({
      kinds: new Set(["code", "decision", "trajectory", "preference"]),
      statuses: new Set(["active", "deprecated"]),
      tags: new Set<string>(),
    });
    setVisibleNodeGroups({
      docs: true,
      tags: true,
      files: true,
      entities: false,
    });
  }, []);

  const focusSeedLabel = useMemo(() => {
    if (!focusSeedId || !graph) return "";
    const node = graph.nodes.find((n) => n.id === focusSeedId);
    return node?.label || focusSeedId;
  }, [focusSeedId, graph]);

  if (isLoading) {
    return (
      <Flex align="center" justify="center" height="100%">
        <Text>Loading graph...</Text>
      </Flex>
    );
  }

  if (error) {
    return (
      <Flex align="center" justify="center" height="100%">
        <Text color="red">Error loading graph</Text>
      </Flex>
    );
  }

  if (!graph) {
    return null;
  }

  return (
    <div className={styles.container}>
      <div className={styles.graphContainer}>
        <div className={styles.modeHeader}>
          <span className={styles.modeIndicator}>
            {mode === "overview" ? "📊 Overview" : `🔍 Focus: ${focusSeedLabel}`}
          </span>
          
          {mode === "focus" && (
            <div className={styles.focusControls}>
              <div className={styles.depthToggle}>
                <button
                  className={`${styles.depthButton} ${focusDepth === 1 ? styles.active : ""}`}
                  onClick={() => setFocusDepth(1)}
                >
                  1-hop
                </button>
                <button
                  className={`${styles.depthButton} ${focusDepth === 2 ? styles.active : ""}`}
                  onClick={() => setFocusDepth(2)}
                >
                  2-hop
                </button>
              </div>
              <button className={styles.backButton} onClick={handleBackToOverview}>
                ← Back
              </button>
            </div>
          )}
        </div>

        {filteredNodes.length === 0 ? (
          <div className={styles.emptyState}>
            <div className={styles.emptyStateIcon}>🔍</div>
            <div className={styles.emptyStateText}>
              <p>No nodes match the current filters</p>
              <button className={styles.resetButton} onClick={resetFilters}>
                Reset Filters
              </button>
            </div>
          </div>
        ) : (
          <div style={{ flex: 1, minWidth: 0, minHeight: 0, display: "flex", overflow: "hidden" }}>
            <CytoscapeComponent
              elements={elements}
              style={{ width: "100%", height: "100%" }}
              stylesheet={stylesheet}
              cy={(cy: any) => {
                cyRef.current = cy;
              }}
            />
          </div>
        )}
      </div>

      <div className={styles.sidebar}>
        <div className={styles.filterSection}>
          <div className={styles.filterTitle}>
            {mode === "overview" ? "Document Types" : "View Mode"}
          </div>
          {mode === "overview" ? (
            <div className={styles.filterOptions}>
              {["code", "decision", "trajectory", "preference"].map((kind) => (
                <label key={kind} className={styles.filterCheckbox}>
                  <Checkbox
                    checked={filters.kinds.has(kind)}
                    onCheckedChange={() => handleKindToggle(kind)}
                  />
                  <Text size="2">{kind}</Text>
                </label>
              ))}
            </div>
          ) : (
            <div className={styles.filterOptions}>
              <label className={styles.filterCheckbox}>
                <Checkbox
                  checked={visibleNodeGroups.tags}
                  onCheckedChange={() => handleNodeGroupToggle("tags")}
                />
                <Text size="2">Tags</Text>
              </label>
              <label className={styles.filterCheckbox}>
                <Checkbox
                  checked={visibleNodeGroups.files}
                  onCheckedChange={() => handleNodeGroupToggle("files")}
                />
                <Text size="2">Files</Text>
              </label>
              <label className={styles.filterCheckbox}>
                <Checkbox
                  checked={visibleNodeGroups.entities}
                  onCheckedChange={() => handleNodeGroupToggle("entities")}
                />
                <Text size="2">Entities</Text>
              </label>
            </div>
          )}
        </div>

        {graph.stats && (
          <div className={styles.filterSection}>
            <div className={styles.filterTitle}>Statistics</div>
            <div className={styles.statsGrid}>
              <div className={styles.statItem}>
                <div className={styles.statLabel}>Documents</div>
                <div className={styles.statValue}>{graph.stats.doc_count}</div>
              </div>
              <div className={styles.statItem}>
                <div className={styles.statLabel}>Tags</div>
                <div className={styles.statValue}>{graph.stats.tag_count}</div>
              </div>
              <div className={styles.statItem}>
                <div className={styles.statLabel}>Files</div>
                <div className={styles.statValue}>{graph.stats.file_count}</div>
              </div>
              <div className={styles.statItem}>
                <div className={styles.statLabel}>Edges</div>
                <div className={styles.statValue}>{graph.stats.edge_count}</div>
              </div>
            </div>
          </div>
        )}

        {selectedNodeData && (
          <div className={styles.nodeDetails}>
            <div className={styles.nodeDetailsTitle}>
              {selectedNodeData.label}
            </div>
            <div className={styles.nodeDetailsContent}>
              <Text size="1" color="gray">
                Type: {selectedNodeData.node_type}
              </Text>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
