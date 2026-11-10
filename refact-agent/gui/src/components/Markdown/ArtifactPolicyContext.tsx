import { createContext } from "react";
import type { ReviewArtifactTheme } from "./artifactPolicy";

export type ReviewSlotPreview = {
  id: string;
  title: string;
  text: string;
  available: boolean;
};

export type ArtifactPolicy = {
  page?: {
    previews: ReviewSlotPreview[];
    onAction: (action: "reveal" | "comment", id: string) => void;
  };
  mode: "review-seamless";
  title?: string;
  bindingKey?: string;
  theme?: ReviewArtifactTheme;
  revealAnchorCount?: number;
  onRevealAnchor?: (index: number) => void;
};

export const ArtifactPolicyContext = createContext<ArtifactPolicy | null>(null);
export const ArtifactPolicyProvider = ArtifactPolicyContext.Provider;
