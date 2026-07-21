import type { MCPServer } from "../../services/refact/mcpMarketplace";

/**
 * Recipe env keys with an empty default value are required (they are almost
 * always credentials). Mirrors the engine's `compute_missing_required_env`
 * gating: installing with these still empty returns HTTP 422.
 */
export function requiredEnvKeys(server: MCPServer): string[] {
  return Object.entries(server.install_recipe.env ?? {})
    .filter(([, value]) => !value || value.trim() === "")
    .map(([key]) => key)
    .sort();
}
