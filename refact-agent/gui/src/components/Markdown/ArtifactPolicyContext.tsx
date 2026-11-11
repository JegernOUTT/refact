import { createContext } from "react";
import type { ReviewArtifactTheme } from "./artifactPolicy";

export type ReviewSlotLine = {
  n: number | null;
  kind: "ctx" | "add" | "del" | "gap";
  text: string;
  /** Optional tint: `[kind, text]` spans whose texts join back to `text`. */
  tokens?: [string, string][];
};

export type ReviewSlotPreview = {
  id: string;
  title: string;
  text: string;
  available: boolean;
  lines?: ReviewSlotLine[];
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
