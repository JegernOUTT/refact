import type {
  PrivacyToolAccess,
  PrivacyZone,
} from "../../services/refact/privacy";

export const SHELL_BEHAVIOR_OPTIONS = [
  { value: "withhold", label: "Withhold output" },
  { value: "ask", label: "Ask first" },
  { value: "deny", label: "Deny command" },
];

export function isCatchAllZone(zone: PrivacyZone) {
  return zone.patterns.some(
    (pattern) => pattern === "*" || pattern === "**" || pattern === "**/*",
  );
}

export function zoneAllowsDestination(
  zone: PrivacyZone,
  destinationId: string,
) {
  return zone.send_to.includes("*") || zone.send_to.includes(destinationId);
}

export function mcpAllowedForProvider(
  toolAccess: PrivacyToolAccess,
  providerId: string,
  server: string,
) {
  const entry = toolAccess.providers[providerId];
  if (!entry) return true;
  return entry.mcp.includes("*") || entry.mcp.includes(server);
}
