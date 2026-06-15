import type { KnowledgeGraphNode } from "../../services/refact/types";

export function isActiveKnowledgeDocNode(node: KnowledgeGraphNode): boolean {
  const nodeType = node.node_type;
  const isDocNode = nodeType === "doc" || nodeType.startsWith("doc_");

  if (!isDocNode) return false;

  const kind = nodeType.replace("doc_", "").toLowerCase();
  return kind !== "deprecated" && kind !== "archived" && kind !== "trajectory";
}
