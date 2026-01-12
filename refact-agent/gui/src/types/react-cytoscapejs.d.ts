declare module "react-cytoscapejs" {
  import type Cytoscape from "cytoscape";
  import type { CSSProperties } from "react";

  export interface CytoscapeComponentProps {
    elements: {
      data: Record<string, unknown>;
      group?: "nodes" | "edges";
    }[];
    style?: CSSProperties;
    stylesheet?: Cytoscape.StylesheetStyle[];
    layout?: Cytoscape.LayoutOptions;
    cy?: (cy: Cytoscape.Core) => void;
    className?: string;
  }

  const CytoscapeComponent: React.FC<CytoscapeComponentProps>;
  export default CytoscapeComponent;
}
