import { createContext } from "react";
import type { ReviewArtifactTheme } from "./artifactPolicy";

export type ArtifactPolicy = {
  mode: "review-seamless";
  title?: string;
  bindingKey?: string;
  theme?: ReviewArtifactTheme;
  revealAnchorCount?: number;
  onRevealAnchor?: (index: number) => void;
};

export const ArtifactPolicyContext = createContext<ArtifactPolicy | null>(null);
export const ArtifactPolicyProvider = ArtifactPolicyContext.Provider;
