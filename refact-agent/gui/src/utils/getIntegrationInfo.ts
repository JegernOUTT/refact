import { toPascalCase } from "./toPascalCase";

const INTEGRATION_ACRONYMS: Record<string, string | undefined> = {
  mcp: "MCP",
  http: "HTTP",
  https: "HTTPS",
  api: "API",
  cli: "CLI",
  url: "URL",
  id: "ID",
  ci: "CI",
  sdk: "SDK",
  ai: "AI",
  db: "DB",
};

export const humanizeIntegrationName = (integrationName: string): string =>
  integrationName
    .split(/[_\-\s]+/)
    .map(
      (part) => INTEGRATION_ACRONYMS[part.toLowerCase()] ?? toPascalCase(part),
    )
    .join(" ");

export const getIntegrationInfo = (integrationName: string) => {
  const isMCPSse = integrationName.startsWith("mcp_sse");
  const isMCPHttp = integrationName.startsWith("mcp_http");
  const isMCPStdio =
    !integrationName.startsWith("mcp_sse") &&
    !integrationName.startsWith("mcp_http") &&
    integrationName.includes("mcp");
  const isCmdline = integrationName.startsWith("cmdline");
  const isService = integrationName.startsWith("service");

  const getDisplayName = () => {
    if (!integrationName.includes("TEMPLATE")) {
      return humanizeIntegrationName(integrationName);
    }
    if (isCmdline) return "Command-line Tool";
    if (isService) return "Command-line Service";
    if (isMCPSse) return "MCP (Connect to SSE)";
    if (isMCPHttp) return "MCP (Streamable HTTP)";
    if (isMCPStdio) return "MCP (Run via stdio)";
    return "";
  };

  return {
    isMCP: isMCPSse || isMCPHttp || isMCPStdio,
    isCmdline,
    isService,
    displayName: getDisplayName(),
  };
};
