import type { DesignSurfaceState } from "./designSlice";

export type DesignRendererKind =
  | "empty"
  | "live-interactive"
  | "live-basic"
  | "browser-frame"
  | "artifact"
  | "reference";

export const DEFAULT_DESIGN_FALLBACK_REASON =
  "The preview could not be embedded because the host may deny framing with X-Frame-Options or Content-Security-Policy frame-ancestors.";

export const HOST_ORIGIN_FALLBACK_REASON =
  "The preview URL shares the Design host origin. It was not embedded because same-origin content could access host APIs; Design is using the isolated CDP browser instead.";

export function resolveDesignRenderer(
  state: DesignSurfaceState,
): DesignRendererKind {
  if (state.source === "artifact") return "artifact";
  if (state.source === "reference") return "reference";
  if (!state.liveUrl) return "empty";
  if (state.liveStatus === "blocked") return "browser-frame";
  if (state.liveStatus === "interactive") return "live-interactive";
  return "live-basic";
}

export function resolveDesignFallbackReason(state: DesignSurfaceState): string {
  const reason = state.fallbackReason?.trim();
  return reason?.length ? reason : DEFAULT_DESIGN_FALLBACK_REASON;
}
