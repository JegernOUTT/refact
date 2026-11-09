import { createElement, type ReactElement } from "react";
import {
  isAppIconArtwork,
  type AppIconArtwork,
  type AppIconGeometry,
} from "../../services/refact/apps";
import { appRailIcon } from "./appRailIcon";
import styles from "./AppIcon.module.css";

type Props = {
  icon?: string | null;
  artwork?: AppIconArtwork | null;
  size?: number;
  className?: string;
};

const attributeNames: Record<string, string | undefined> = {
  "stroke-width": "strokeWidth",
  "stroke-miterlimit": "strokeMiterlimit",
  "stroke-linecap": "strokeLinecap",
  "stroke-linejoin": "strokeLinejoin",
  "fill-rule": "fillRule",
  "clip-rule": "clipRule",
  "fill-opacity": "fillOpacity",
  "stroke-opacity": "strokeOpacity",
};
function geometry(node: AppIconGeometry, key: number): ReactElement {
  const props: Record<string, string | number> = { key };
  for (const [name, value] of Object.entries(node.attrs)) {
    props[attributeNames[name] ?? name] = value;
  }
  // The caller validates every tag and attribute first. No markup, URL, style,
  // event handler, id or class from the bundle is ever passed to React.
  return createElement(node.tag, props, node.children.map(geometry));
}

/** Decorative model artwork; the adjacent app name supplies the accessible label. */
export function AppIcon({ icon, artwork, size = 28, className }: Props) {
  const dimension = Number.isFinite(size)
    ? Math.min(96, Math.max(12, size))
    : 28;
  if (!isAppIconArtwork(artwork)) {
    const Fallback = appRailIcon(icon);
    return (
      <Fallback
        width={dimension}
        height={dimension}
        strokeWidth={1.5}
        aria-hidden="true"
        focusable="false"
        className={className}
      />
    );
  }
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      viewBox={artwork.view_box.join(" ")}
      width={dimension}
      height={dimension}
      className={[styles.artwork, className].filter(Boolean).join(" ")}
      aria-hidden="true"
      focusable="false"
    >
      {artwork.nodes.map(geometry)}
    </svg>
  );
}
