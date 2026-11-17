import type { Element, ElementContent, Root, RootContent } from "hast";
import { parseArtifactLink } from "../../utils/artifactLinks";

export const ARTIFACT_CARD_HREF_PROPERTY = "dataArtifactCardHref";
export const ARTIFACT_CARD_TITLE_PROPERTY = "dataArtifactCardTitle";

type Parent = Root | Element;

function isWhitespaceText(node: ElementContent): boolean {
  return (
    node.type === "comment" || (node.type === "text" && !node.value.trim())
  );
}

function textContent(node: ElementContent): string {
  if (node.type === "text") return node.value;
  if (node.type === "element") return node.children.map(textContent).join("");
  return "";
}

function artifactAnchor(node: ElementContent): Element | null {
  if (node.type !== "element" || node.tagName !== "a") return null;
  const href = node.properties.href;
  return typeof href === "string" && parseArtifactLink(href) ? node : null;
}

function soleArtifactAnchor(nodes: ElementContent[]): Element | null {
  const meaningful = nodes.filter((node) => !isWhitespaceText(node));
  if (meaningful.length !== 1) return null;
  return artifactAnchor(meaningful[0]);
}

function artifactCard(anchor: Element): Element {
  return {
    type: "element",
    tagName: "div",
    properties: {
      [ARTIFACT_CARD_HREF_PROPERTY]: String(anchor.properties.href),
      [ARTIFACT_CARD_TITLE_PROPERTY]: textContent(anchor).trim(),
    },
    children: anchor.children,
  };
}

function isArtifactCard(node: ElementContent): boolean {
  return (
    node.type === "element" &&
    node.tagName === "div" &&
    typeof node.properties[ARTIFACT_CARD_HREF_PROPERTY] === "string"
  );
}

function splitLines(
  children: ElementContent[],
  wrap: (nodes: ElementContent[]) => ElementContent[],
): ElementContent[] | null {
  const lines: ElementContent[][] = [[]];
  const breaks: Element[] = [];
  for (const child of children) {
    if (child.type === "element" && child.tagName === "br") {
      breaks.push(child);
      lines.push([]);
    } else {
      lines[lines.length - 1].push(child);
    }
  }
  const anchors = lines.map(soleArtifactAnchor);
  if (anchors.every((anchor) => anchor === null)) return null;

  const result: ElementContent[] = [];
  let pending: ElementContent[] = [];
  const flush = () => {
    if (pending.some((node) => !isWhitespaceText(node))) {
      result.push(...wrap(pending));
    }
    pending = [];
  };
  lines.forEach((line, index) => {
    const anchor = anchors[index];
    if (anchor) {
      flush();
      result.push(artifactCard(anchor));
      return;
    }
    if (pending.length > 0 && index > 0) pending.push(breaks[index - 1]);
    pending.push(...line);
  });
  flush();
  return result;
}

function splitParagraph(paragraph: Element): ElementContent[] | null {
  return splitLines(paragraph.children, (nodes) => [
    { ...paragraph, children: nodes },
  ]);
}

function transformListItem(item: Element): void {
  const meaningful = item.children.filter((node) => !isWhitespaceText(node));
  if (meaningful.length === 1 && isArtifactCard(meaningful[0])) {
    item.children = [meaningful[0]];
    return;
  }
  const tight = !item.children.some(
    (node) => node.type === "element" && node.tagName === "p",
  );
  if (!tight) return;
  const split = splitLines(item.children, (nodes) => nodes);
  if (split) item.children = split;
}

function transformChildren(parent: Parent): void {
  const next: RootContent[] = [];
  for (const child of parent.children) {
    if (child.type !== "element") {
      next.push(child);
      continue;
    }
    transformChildren(child);
    if (child.tagName === "p") {
      const split = splitParagraph(child);
      if (split) {
        next.push(...split);
        continue;
      }
    }
    if (child.tagName === "li") transformListItem(child);
    next.push(child);
  }
  parent.children = next as typeof parent.children;
}

/**
 * Lifts every paragraph or tight-list-item line (delimited by paragraph
 * boundaries or `<br>`) that holds nothing but one artifact link into a block
 * `div` carrying the link as data attributes, so it renders as a preview card.
 */
export function artifactCardFromNode(
  node: Element | undefined,
): { href: string; title: string } | null {
  if (!node) return null;
  const href = node.properties[ARTIFACT_CARD_HREF_PROPERTY];
  const title = node.properties[ARTIFACT_CARD_TITLE_PROPERTY];
  if (typeof href !== "string" || !parseArtifactLink(href)) return null;
  return { href, title: typeof title === "string" ? title : "" };
}

export function rehypeArtifactCards() {
  return (tree: Root) => {
    transformChildren(tree);
  };
}
