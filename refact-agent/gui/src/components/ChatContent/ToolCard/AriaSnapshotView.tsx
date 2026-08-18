import { ChevronRight, Search } from "lucide-react";
import { useMemo, useState } from "react";
import { Box, Flex, Text } from "@radix-ui/themes";

import type {
  BrowserAriaSnapshotNode,
  BrowserSnapshotBox,
  BrowserSnapshotGeneration,
} from "../../../services/refact/browser";
import { Badge, Button, FieldText, Icon } from "../../ui";
import { ShikiCodeBlock } from "../../Markdown";
import styles from "./AriaSnapshotView.module.css";

const LARGE_SNAPSHOT_NODE_LIMIT = 40;
const SUPPORTED_STATES = new Set([
  "checked",
  "disabled",
  "expanded",
  "level",
  "pressed",
  "selected",
]);

interface AriaTreeNode {
  id: string;
  role: string;
  name: string | null;
  states: string[];
  reference: string | null;
  box: BrowserSnapshotBox | null;
  properties: Record<string, string>;
  children: AriaTreeNode[];
  searchText: string;
}

export interface AriaSnapshotViewProps {
  yaml: string;
  nodes?: BrowserAriaSnapshotNode[];
  generation?: BrowserSnapshotGeneration | null;
}

interface MutableTreeNode extends Omit<AriaTreeNode, "searchText"> {
  searchText?: string;
}

interface ParsedNodeHeader {
  role: string;
  name: string | null;
  states: string[];
  reference: string | null;
  text: string | null;
}

function indentationDepth(line: string): number | null {
  const prefix = /^\s*/.exec(line)?.[0] ?? "";
  if (prefix.includes("\t") || prefix.length % 2 !== 0) {
    return null;
  }
  return prefix.length / 2;
}

function unquoteYamlValue(value: string): string {
  const trimmed = value.trim();
  if (trimmed.startsWith('"') && trimmed.endsWith('"')) {
    try {
      const parsed = JSON.parse(trimmed) as unknown;
      return typeof parsed === "string" ? parsed : trimmed;
    } catch {
      return trimmed.slice(1, -1);
    }
  }
  if (trimmed.startsWith("'") && trimmed.endsWith("'")) {
    return trimmed.slice(1, -1).replace(/''/g, "'");
  }
  return trimmed;
}

function splitNodeHeader(
  value: string,
): { header: string; text: string | null } | null {
  let inDoubleQuote = false;
  let inSingleQuote = false;
  let escaped = false;

  for (let index = 0; index < value.length; index++) {
    const character = value[index];
    if (escaped) {
      escaped = false;
      continue;
    }
    if (character === "\\" && inDoubleQuote) {
      escaped = true;
      continue;
    }
    if (character === '"' && !inSingleQuote) {
      inDoubleQuote = !inDoubleQuote;
      continue;
    }
    if (character === "'" && !inDoubleQuote) {
      inSingleQuote = !inSingleQuote;
      continue;
    }
    if (character === ":" && !inDoubleQuote && !inSingleQuote) {
      const rest = value.slice(index + 1);
      if (rest === "" || rest.startsWith(" ")) {
        return {
          header: value.slice(0, index),
          text: rest.trim() ? unquoteYamlValue(rest) : null,
        };
      }
    }
  }

  return inDoubleQuote || inSingleQuote ? null : { header: value, text: null };
}

function parseQuotedName(value: string): { name: string; rest: string } | null {
  if (value.startsWith('"')) {
    let escaped = false;
    for (let index = 1; index < value.length; index++) {
      const character = value[index];
      if (escaped) {
        escaped = false;
      } else if (character === "\\") {
        escaped = true;
      } else if (character === '"') {
        const encoded = value.slice(0, index + 1);
        try {
          const parsed = JSON.parse(encoded) as unknown;
          if (typeof parsed !== "string") return null;
          return { name: parsed, rest: value.slice(index + 1).trimStart() };
        } catch {
          return null;
        }
      }
    }
    return null;
  }

  if (value.startsWith("'")) {
    for (let index = 1; index < value.length; index++) {
      if (value[index] !== "'") continue;
      if (value[index + 1] === "'") {
        index++;
        continue;
      }
      return {
        name: value.slice(1, index).replace(/''/g, "'"),
        rest: value.slice(index + 1).trimStart(),
      };
    }
    return null;
  }

  return null;
}

function unquoteYamlKey(value: string): string | null {
  if (!value.startsWith("'")) return value;
  if (!value.endsWith("'")) return null;
  return value.slice(1, -1).replace(/''/g, "'");
}

function parseNodeHeader(value: string): ParsedNodeHeader | null {
  const split = splitNodeHeader(value);
  if (split === null) return null;
  const header = unquoteYamlKey(split.header);
  if (header === null) return null;
  const { text } = split;

  const roleMatch = /^([^\s[]+)/.exec(header);
  if (!roleMatch) return null;
  const role = roleMatch[1];
  let rest = header.slice(role.length).trimStart();
  let name: string | null = null;

  if (rest.startsWith('"') || rest.startsWith("'")) {
    const parsedName = parseQuotedName(rest);
    if (!parsedName) return null;
    name = parsedName.name;
    rest = parsedName.rest;
  }

  const attributes: string[] = [];
  while (rest.length > 0) {
    const attributeMatch = /^\[([^\]]+)\](?:\s+|$)/.exec(rest);
    if (!attributeMatch) return null;
    attributes.push(attributeMatch[1]);
    rest = rest.slice(attributeMatch[0].length);
  }

  let reference: string | null = null;
  const states: string[] = [];
  for (const attribute of attributes) {
    const separator = attribute.indexOf("=");
    const key = separator >= 0 ? attribute.slice(0, separator) : attribute;
    const valuePart = separator >= 0 ? attribute.slice(separator + 1) : null;
    if (key === "ref" && valuePart) {
      reference = valuePart;
    } else if (SUPPORTED_STATES.has(key)) {
      states.push(valuePart ? `${key}=${valuePart}` : key);
    }
  }

  return { role, name, states, reference, text };
}

function buildSearchText(node: MutableTreeNode): string {
  return [
    node.role,
    node.name ?? "",
    node.reference ?? "",
    ...node.states,
    ...Object.entries(node.properties).flatMap(([key, value]) => [key, value]),
  ]
    .join(" ")
    .toLocaleLowerCase();
}

function parseSnapshot(yaml: string): AriaTreeNode[] | null {
  if (!yaml.trim()) return null;

  const roots: MutableTreeNode[] = [];
  const stack: MutableTreeNode[] = [];
  const lines = yaml.split(/\r?\n/);
  let nodeSequence = 0;

  for (let lineIndex = 0; lineIndex < lines.length; lineIndex++) {
    const line = lines[lineIndex];
    if (!line.trim()) continue;
    const depth = indentationDepth(line);
    if (depth === null) return null;
    const content = line.trimStart();
    if (!content.startsWith("- ")) return null;
    const item = content.slice(2);

    if (item.startsWith("/")) {
      if (depth === 0 || stack.length < depth) return null;
      const propertyMatch = /^\/([a-z_]+):\s*(.*)$/.exec(item);
      if (!propertyMatch) return null;
      const parent = stack[depth - 1];
      parent.properties[propertyMatch[1]] = unquoteYamlValue(propertyMatch[2]);
      continue;
    }

    const header = parseNodeHeader(item);
    if (!header) return null;
    if (depth > stack.length) return null;

    const node: MutableTreeNode = {
      id: `aria-${nodeSequence++}-${lineIndex}`,
      role: header.role,
      name: header.name,
      states: header.states,
      reference: header.reference,
      box: null,
      properties: header.text === null ? {} : { text: header.text },
      children: [],
    };

    if (depth === 0) {
      roots.push(node);
    } else {
      const parent = stack[depth - 1];
      parent.children.push(node as AriaTreeNode);
    }
    stack.length = depth;
    stack[depth] = node;
  }

  if (roots.length === 0) return null;

  const finalize = (node: MutableTreeNode): AriaTreeNode => {
    node.children = node.children.map((child) => finalize(child));
    return { ...node, searchText: buildSearchText(node) } as AriaTreeNode;
  };
  return roots.map(finalize);
}

function countNodes(nodes: AriaTreeNode[]): number {
  return nodes.reduce(
    (count, node) => count + 1 + countNodes(node.children),
    0,
  );
}

function enrichReferences(
  roots: AriaTreeNode[],
  metadata: BrowserAriaSnapshotNode[],
): AriaTreeNode[] {
  if (metadata.length === 0) return roots;

  const byRoleAndName = new Map<string, string[]>();
  const boxesByRoleAndName = new Map<string, BrowserSnapshotBox[]>();
  for (const item of metadata) {
    const key = `${item.role}\u0000${item.name ?? ""}`;
    if (item.ref) {
      const references = byRoleAndName.get(key) ?? [];
      references.push(item.ref);
      byRoleAndName.set(key, references);
    }
    if (item.box) {
      const boxes = boxesByRoleAndName.get(key) ?? [];
      boxes.push(item.box);
      boxesByRoleAndName.set(key, boxes);
    }
  }

  const cursors = new Map<string, number>();
  const boxCursors = new Map<string, number>();
  const visit = (node: AriaTreeNode): AriaTreeNode => {
    const key = `${node.role}\u0000${node.name ?? ""}`;
    let reference = node.reference;
    if (!reference) {
      const references = byRoleAndName.get(key);
      const cursor = cursors.get(key) ?? 0;
      reference = references?.[cursor] ?? null;
      if (reference) cursors.set(key, cursor + 1);
    }
    let box = node.box;
    if (!box) {
      const boxes = boxesByRoleAndName.get(key);
      const boxCursor = boxCursors.get(key) ?? 0;
      box = boxes?.[boxCursor] ?? null;
      if (box) boxCursors.set(key, boxCursor + 1);
    }
    const enriched = {
      ...node,
      reference,
      box,
      children: node.children.map(visit),
    };
    return { ...enriched, searchText: buildSearchText(enriched) };
  };

  return roots.map(visit);
}

function formatBox(box: BrowserSnapshotBox): string {
  return `${box.width}×${box.height}@${box.x},${box.y}`;
}

function filterTree(nodes: AriaTreeNode[], query: string): AriaTreeNode[] {
  const needle = query.trim().toLocaleLowerCase();
  if (!needle) return nodes;

  return nodes.flatMap((node) => {
    const children = filterTree(node.children, needle);
    return node.searchText.includes(needle) || children.length > 0
      ? [{ ...node, children }]
      : [];
  });
}

function limitTree(
  nodes: AriaTreeNode[],
  limit: number,
): { nodes: AriaTreeNode[]; visibleCount: number } {
  let remaining = limit;
  let visibleCount = 0;

  const visit = (node: AriaTreeNode): AriaTreeNode | null => {
    if (remaining === 0) return null;
    remaining--;
    visibleCount++;
    const children = node.children
      .map(visit)
      .filter((child): child is AriaTreeNode => child !== null);
    return { ...node, children };
  };

  return {
    nodes: nodes
      .map(visit)
      .filter((node): node is AriaTreeNode => node !== null),
    visibleCount,
  };
}

function TreeNode({ node, query }: { node: AriaTreeNode; query: string }) {
  const hasChildren = node.children.length > 0;
  const [expanded, setExpanded] = useState(true);
  const searching = Boolean(query);
  const isExpanded = searching || expanded;
  const matches = searching && node.searchText.includes(query);

  return (
    <li
      className={styles.treeItem}
      data-has-ref={Boolean(node.reference)}
      role="treeitem"
    >
      <Box className={styles.nodeRow} data-match={matches}>
        {hasChildren ? (
          <button
            aria-label={`${isExpanded ? "Collapse" : "Expand"} ${node.role}${
              node.name ? ` ${node.name}` : ""
            }`}
            aria-expanded={isExpanded}
            className={styles.nodeToggle}
            disabled={searching}
            onClick={() => setExpanded((current) => !current)}
            type="button"
          >
            <Icon icon={ChevronRight} size="sm" tone="muted" />
          </button>
        ) : (
          <span className={styles.nodeSpacer} />
        )}
        <Flex align="center" gap="1" wrap="wrap" className={styles.nodeContent}>
          <Text as="span" className={styles.role} weight="bold">
            {node.role}
          </Text>
          {node.name !== null && (
            <Text as="span" className={styles.name}>
              “{node.name}”
            </Text>
          )}
          {node.states.map((state) => (
            <Badge key={state} size="xs" tone="muted" variant="soft">
              {state}
            </Badge>
          ))}
          {node.reference && (
            <Badge
              className={styles.refBadge}
              data-testid="aria-ref-badge"
              size="xs"
              tone="accent"
              variant="outline"
            >
              ref={node.reference}
            </Badge>
          )}
          {node.box && (
            <Badge
              className={styles.boxBadge}
              data-testid="aria-box-badge"
              size="xs"
              title={`x ${node.box.x}, y ${node.box.y}, ${node.box.width}×${node.box.height}`}
              tone="muted"
              variant="outline"
            >
              {formatBox(node.box)}
            </Badge>
          )}
        </Flex>
      </Box>
      {Object.entries(node.properties).length > 0 && (
        <Flex direction="column" gap="1" className={styles.properties}>
          {Object.entries(node.properties).map(([key, value]) => (
            <Text as="span" key={key} className={styles.property}>
              /{key}: {value}
            </Text>
          ))}
        </Flex>
      )}
      {hasChildren && isExpanded && (
        <Tree nodes={node.children} query={query} />
      )}
    </li>
  );
}

function Tree({ nodes, query }: { nodes: AriaTreeNode[]; query: string }) {
  return (
    <ul className={styles.tree} role="tree">
      {nodes.map((node) => (
        <TreeNode key={node.id} node={node} query={query} />
      ))}
    </ul>
  );
}

export function AriaSnapshotView({
  yaml,
  nodes = [],
  generation = null,
}: AriaSnapshotViewProps) {
  const parsed = useMemo(() => parseSnapshot(yaml), [yaml]);
  const enriched = useMemo(
    () => (parsed ? enrichReferences(parsed, nodes) : null),
    [parsed, nodes],
  );
  const [query, setQuery] = useState("");
  const [showAll, setShowAll] = useState(false);

  if (!enriched) {
    return (
      <Box className={styles.fallback} data-testid="aria-snapshot-fallback">
        <ShikiCodeBlock showLineNumbers={false}>{yaml}</ShikiCodeBlock>
      </Box>
    );
  }

  const filtered = filterTree(enriched, query);
  const totalCount = countNodes(enriched);
  const isLarge = totalCount > LARGE_SNAPSHOT_NODE_LIMIT;
  const normalizedQuery = query.trim().toLocaleLowerCase();
  const limited = limitTree(filtered, LARGE_SNAPSHOT_NODE_LIMIT);
  const visibleRoots =
    normalizedQuery || showAll || !isLarge ? filtered : limited.nodes;
  const hiddenCount = Math.max(0, totalCount - limited.visibleCount);
  const matchCount = countNodes(filtered);
  const canShowMore = !normalizedQuery && !showAll && hiddenCount > 0;

  return (
    <Box className={styles.view} data-testid="aria-snapshot-view">
      <Flex align="center" gap="2" className={styles.toolbar}>
        <span className={styles.searchIcon}>
          <Icon icon={Search} size="sm" tone="muted" />
        </span>
        <FieldText
          aria-label="Filter ARIA snapshot"
          className={styles.searchInput}
          onChange={setQuery}
          placeholder="Filter roles, names, states, refs…"
          value={query}
        />
        {generation && (
          <Text
            as="span"
            className={styles.generation}
            data-testid="aria-generation"
          >
            gen d{generation.document_generation}/f
            {generation.frame_generation}
          </Text>
        )}
        <Text as="span" className={styles.nodeCount}>
          {query.trim()
            ? `${matchCount} match${matchCount === 1 ? "" : "es"}`
            : `${totalCount} nodes`}
        </Text>
      </Flex>
      <Box className={styles.scrollRegion}>
        {visibleRoots.length > 0 ? (
          <Tree nodes={visibleRoots} query={normalizedQuery} />
        ) : (
          <Text as="p" className={styles.emptyState}>
            No snapshot nodes match “{query}”.
          </Text>
        )}
        {canShowMore && (
          <Button
            className={styles.showAllButton}
            onClick={() => setShowAll(true)}
            size="sm"
            variant="plain"
          >
            Show {hiddenCount} more nodes
          </Button>
        )}
      </Box>
    </Box>
  );
}
