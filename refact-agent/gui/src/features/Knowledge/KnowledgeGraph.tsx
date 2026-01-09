import { useEffect, useRef, useState } from "react";
import CytoscapeComponent from "react-cytoscapejs";
import type Cytoscape from "cytoscape";
import fcose from "cytoscape-fcose";
import { Flex, Text, Checkbox } from "@radix-ui/themes";
import { useGetKnowledgeGraphQuery } from "../../services/refact/knowledgeGraphApi";
import { useKnowledgeGraphTheme } from "./useKnowledgeGraphTheme";
import styles from "./KnowledgeGraph.module.css";
import type { KnowledgeGraphEdge } from "../../services/refact/types";

// @ts-ignore - fcose is a Cytoscape extension
const CytoscapeLib = require("cytoscape");
CytoscapeLib.use(fcose);

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

export function KnowledgeGraph() {
  const { data: graph, isLoading, error } = useGetKnowledgeGraphQuery(undefined);
  const { colors } = useKnowledgeGraphTheme();
  const cyRef = useRef<Cytoscape.Core | null>(null);
  const [selectedNode, setSelectedNode] = useState<string | null>(null);
  const [filters, setFilters] = useState<FilterState>({
    kinds: new Set(["code", "decision", "trajectory", "preference"]),
    statuses: new Set(["active", "deprecated"]),
    tags: new Set<string>(),
  });

  useEffect(() => {
    if (cyRef.current) {
      cyRef.current.on("tap", "node", (e: any) => {
        const nodeId = e.target.id();
        setSelectedNode(nodeId);
      });

      cyRef.current.on("tap", (e: any) => {
        if (e.target === cyRef.current) {
          setSelectedNode(null);
        }
      });
    }
  }, []);

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

  const computeDegree = (
    nodeId: string,
    edges: KnowledgeGraphEdge[],
  ): number => {
    return edges.filter((e) => e.source === nodeId || e.target === nodeId)
      .length;
  };

  const filteredNodes = graph.nodes.filter((node) => {
    const nodeType = node.node_type.toLowerCase();
    if (nodeType.includes("doc_")) {
      const kind = nodeType.replace("doc_", "");
      return filters.kinds.has(kind);
    }
    return true;
  });

  const filteredNodeIds = new Set(filteredNodes.map((n) => n.id));
  const filteredEdges = graph.edges.filter(
    (edge) =>
      filteredNodeIds.has(edge.source) && filteredNodeIds.has(edge.target),
  );

  const elements: CytoscapeElement[] = [
    ...filteredNodes.map((node) => ({
      data: {
        id: node.id,
        label: node.label,
        type: node.node_type,
        degree: computeDegree(node.id, filteredEdges),
      },
      group: "nodes" as const,
    })),
    ...filteredEdges.map((edge) => ({
      data: {
        id: `${edge.source}-${edge.target}`,
        source: edge.source,
        target: edge.target,
        label: edge.edge_type,
      },
      group: "edges" as const,
    })),
  ];

  const getColorForType = (type: string): string => {
    if (type.includes("doc_code")) return colors.kind.code;
    if (type.includes("doc_decision")) return colors.kind.decision;
    if (type.includes("doc_trajectory")) return colors.kind.trajectory;
    if (type.includes("doc_preference")) return colors.kind.preference;
    return colors.kind.other;
  };

  const stylesheet: any[] = [
    {
      selector: "node",
      style: {
        "background-color": colors.accent,
        label: "data(label)",
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
    ...filteredNodes.map((node) => ({
      selector: `node[id="${node.id}"]`,
      style: {
        "background-color": getColorForType(node.node_type),
      },
    })),
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

  const selectedNodeData = selectedNode
    ? graph.nodes.find((n) => n.id === selectedNode)
    : null;

  return (
    <div className={styles.container}>
      <CytoscapeComponent
        elements={elements}
        style={{ width: "100%", height: "100%" }}
        stylesheet={stylesheet}
        layout={{
          name: "fcose",
          animationDuration: 500,
          randomize: false,
        } as any}
        cy={(cy: any) => {
          cyRef.current = cy;
        }}
        className={styles.graphContainer}
      />

      <div className={styles.sidebar}>
        <div className={styles.filterSection}>
          <div className={styles.filterTitle}>Document Types</div>
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
