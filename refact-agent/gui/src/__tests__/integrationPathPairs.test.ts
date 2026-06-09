import { describe, expect, it } from "vitest";
import { sortIntegrationPathPairs } from "../components/IntegrationsView/hooks/useIntegrations";

describe("sortIntegrationPathPairs", () => {
  it("keeps config paths paired with their project paths while prioritizing global config", () => {
    const sorted = sortIntegrationPathPairs([
      {
        project_path: "/workspace/project-a",
        integr_config_path: "/workspace/project-a/.refact/integrations.d/mcp_TEMPLATE.yaml",
      },
      {
        project_path: "",
        integr_config_path: "/home/user/.config/refact/integrations.d/mcp_TEMPLATE.yaml",
      },
      {
        project_path: "/workspace/project-b",
        integr_config_path: "/workspace/project-b/.refact/integrations.d/mcp_TEMPLATE.yaml",
      },
    ]);

    expect(sorted).toEqual([
      {
        project_path: "",
        integr_config_path: "/home/user/.config/refact/integrations.d/mcp_TEMPLATE.yaml",
      },
      {
        project_path: "/workspace/project-a",
        integr_config_path: "/workspace/project-a/.refact/integrations.d/mcp_TEMPLATE.yaml",
      },
      {
        project_path: "/workspace/project-b",
        integr_config_path: "/workspace/project-b/.refact/integrations.d/mcp_TEMPLATE.yaml",
      },
    ]);
  });
});
