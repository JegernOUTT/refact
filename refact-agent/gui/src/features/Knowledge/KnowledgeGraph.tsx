import { useEffect, useRef, useState, useMemo } from "react";
import CytoscapeComponent from "react-cytoscapejs";
import cytoscape from "cytoscape";
import type Cytoscape from "cytoscape";
import fcose from "cytoscape-fcose";
import { Flex, Text, Checkbox } from "@radix-ui/themes";
import { useGetKnowledgeGraphQuery } from "../../services/refact/knowledgeGraphApi";
import { useKnowledgeGraphTheme } from "./useKnowledgeGraphTheme";
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
