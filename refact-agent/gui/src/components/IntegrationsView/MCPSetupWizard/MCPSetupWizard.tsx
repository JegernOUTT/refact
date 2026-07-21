import * as RadioGroupPrimitive from "@radix-ui/react-radio-group";
import { FC, useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  useGetAutoNameMutation,
  useWizardProbeMutation,
} from "../../../services/refact/mcpMarketplace";
import type { WizardProbeResponse } from "../../../services/refact/mcpMarketplace";
import { NotConfiguredIntegrationWithIconRecord } from "../../../services/refact";
import { validateSnakeCase } from "../../../utils/validateSnakeCase";
import { createProjectLabelsWithConflictMarkers } from "../../../utils/createProjectLabelsWithConflictMarkers";
import { FieldText, Switch, Button, Surface } from "../../ui";
import { IntegrationPathField } from "../IntermediateIntegration/IntegrationPathField";
import styles from "./MCPSetupWizard.module.css";

type MCPSetupWizardProps = {
  integration: NotConfiguredIntegrationWithIconRecord;
  onSubmit: (
    configPath: string,
    integrName: string,
    initialInput?: { input: string; transport: string },
  ) => void;
};

export const MCPSetupWizard: FC<MCPSetupWizardProps> = ({
  integration,
  onSubmit,
}) => {
  const [input, setInput] = useState("");
  const [suggestedName, setSuggestedName] = useState("");
  const [nameError, setNameError] = useState("");
  const [transport, setTransport] = useState<"stdio" | "http" | "sse">("stdio");
  const [apiPrefix, setApiPrefix] = useState("mcp_stdio_");
  const [detecting, setDetecting] = useState(false);
  const [useSSE, setUseSSE] = useState(false);
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [selectedConfigPath, setSelectedConfigPath] = useState(
    integration.integr_config_path[0] ?? "",
  );

  const [getAutoName] = useGetAutoNameMutation();
  const [wizardProbe, { isLoading: isProbing }] = useWizardProbeMutation();
  const [probeResult, setProbeResult] = useState<WizardProbeResponse | null>(
    null,
  );
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const handleTestConnection = async () => {
    setProbeResult(null);
    try {
      const result = await wizardProbe({ input: input.trim() }).unwrap();
      setProbeResult(result);
    } catch {
      setProbeResult({ transport: "stdio", error: "Probe request failed" });
    }
  };

  const probeSummary = (() => {
    if (!probeResult) return null;
    if (probeResult.error) return `✗ ${probeResult.error}`;
    if (probeResult.transport === "http") {
      if (!probeResult.reachable) return "✗ Server is not reachable";
      if (probeResult.needs_auth && probeResult.oauth_available) {
        return "✓ Reachable — OAuth login will be offered after setup";
      }
      if (probeResult.needs_auth) {
        return "⚠ Reachable, but requires manual authentication";
      }
      return "✓ Server is reachable";
    }
    return probeResult.command_found
      ? `✓ Command found: ${probeResult.resolved_path ?? ""}`
      : "✗ Command not found in PATH";
  })();

  const pathOptions = useMemo(() => {
    return integration.integr_config_path.map((configPath, index) => ({
      configPath,
      projectPath: integration.project_path[index] ?? "",
    }));
  }, [integration.integr_config_path, integration.project_path]);

  const projectLabels = useMemo(() => {
    const validProjectPaths = pathOptions
      .map((option) => option.projectPath)
      .filter((path) => path !== "");
    return createProjectLabelsWithConflictMarkers(validProjectPaths);
  }, [pathOptions]);

  const effectiveTransport = useSSE ? "sse" : transport;
  // Transport detection lives in the engine (/v1/mcp/auto-name); the only
  // local decision is the explicit legacy-SSE override toggle.
  const configPrefix = useSSE ? "mcp_sse_" : apiPrefix;

  const transportLabel =
    effectiveTransport === "stdio"
      ? "Local server (stdio)"
      : effectiveTransport === "sse"
        ? "Remote server (SSE)"
        : "Remote server (HTTP)";

  const handleInputChange = useCallback(
    (value: string) => {
      setInput(value);
      setProbeResult(null);

      if (debounceRef.current) {
        clearTimeout(debounceRef.current);
      }

      if (!value.trim()) {
        setSuggestedName("");
        setTransport("stdio");
        setApiPrefix("mcp_stdio_");
        setDetecting(false);
        return;
      }
      setDetecting(true);

      debounceRef.current = setTimeout(() => {
        void getAutoName({ input: value.trim() })
          .unwrap()
          .then((result) => {
            setSuggestedName(result.suggested_name);
            setTransport(result.transport);
            setApiPrefix(result.config_prefix);
            if (result.transport !== "stdio") {
              setUseSSE(false);
            }
            if (!validateSnakeCase(result.suggested_name)) {
              setNameError("The name must be in snake_case!");
            } else {
              setNameError("");
            }
            setDetecting(false);
          })
          .catch(() => {
            // Offline fallback only: the engine endpoint is the source of
            // truth for detection, this mirrors its trivial URL check.
            const trimmed = value.trim();
            const isUrl =
              trimmed.startsWith("http://") || trimmed.startsWith("https://");
            setTransport(isUrl ? "http" : "stdio");
            setApiPrefix(isUrl ? "mcp_http_" : "mcp_stdio_");
            if (isUrl) {
              setUseSSE(false);
            }
            const fallback = trimmed
              .split(/[^a-z0-9]+/i)
              .filter(Boolean)
              .map((s) => s.toLowerCase())
              .join("_")
              .replace(/^_+|_+$/g, "")
              .slice(0, 40);
            setSuggestedName(fallback || "mcp_server");
            setDetecting(false);
          });
      }, 300);
    },
    [getAutoName],
  );

  useEffect(() => {
    return () => {
      if (debounceRef.current) {
        clearTimeout(debounceRef.current);
      }
    };
  }, []);

  const handleNameChange = (value: string) => {
    setSuggestedName(value);
    if (!validateSnakeCase(value)) {
      setNameError("The name must be in snake_case!");
    } else {
      setNameError("");
    }
  };

  const handleSubmit = () => {
    if (!suggestedName || nameError) return;
    const basePath = selectedConfigPath;
    const configPath = basePath
      .replace(
        /mcp_(?:stdio|sse|http)_TEMPLATE/,
        `${configPrefix}${suggestedName}`,
      )
      .replace(/mcp_TEMPLATE/, `${configPrefix}${suggestedName}`);
    const integrName = `${configPrefix}${suggestedName}`;
    onSubmit(configPath, integrName, {
      input: input.trim(),
      transport: effectiveTransport,
    });
  };

  const canSubmit =
    !!input.trim() && !!suggestedName && !nameError && !detecting;

  return (
    <Surface
      animated="rise"
      className={styles.root}
      radius="card"
      variant="glass"
    >
      <p className={styles.text}>
        Enter the command or URL for your MCP server:
      </p>

      <FieldText
        placeholder="npx -y @modelcontextprotocol/server-github"
        value={input}
        onChange={handleInputChange}
        data-testid="mcp-wizard-input"
      />

      {input.trim() && (
        <div className={styles.detectedStack}>
          <p className={styles.text}>Detected: {transportLabel}</p>

          <div className={styles.nameRow}>
            <p className={styles.text}>Name:</p>
            <div className={styles.nameField}>
              <FieldText
                value={suggestedName}
                onChange={handleNameChange}
                data-testid="mcp-wizard-name"
              />
            </div>
          </div>
          {nameError && <p className={styles.error}>{nameError}</p>}
        </div>
      )}

      <RadioGroupPrimitive.Root
        className={styles.pathGroup}
        name="integr_config_path"
        value={selectedConfigPath}
        onValueChange={setSelectedConfigPath}
      >
        {pathOptions.map(({ configPath, projectPath }) => {
          const shouldPathBeFormatted = projectPath !== "";
          return (
            <label key={configPath}>
              <IntegrationPathField
                configPath={configPath}
                projectPath={projectPath}
                projectLabels={projectLabels}
                shouldBeFormatted={shouldPathBeFormatted}
              />
            </label>
          );
        })}
      </RadioGroupPrimitive.Root>

      {transport === "stdio" && (
        <div>
          <button
            type="button"
            className={styles.advancedToggle}
            onClick={() => setAdvancedOpen((v) => !v)}
          >
            {advancedOpen ? "▼" : "▶"} Advanced: Use SSE transport instead
          </button>
          <div
            className="rf-expand-grid"
            data-open={advancedOpen ? true : undefined}
            data-state={advancedOpen ? "open" : "closed"}
          >
            <div>
              <div className={styles.advancedPanelWrap}>
                <div className={styles.advancedPanel}>
                  <Switch
                    id="use-sse"
                    checked={useSSE}
                    onCheckedChange={setUseSSE}
                    data-testid="mcp-wizard-sse-checkbox"
                  />
                  <label className={styles.text} htmlFor="use-sse">
                    Use SSE transport
                  </label>
                </div>
              </div>
            </div>
          </div>
        </div>
      )}

      <div className={styles.detectedStack}>
        {input.trim() && (
          <Button
            type="button"
            variant="ghost"
            disabled={isProbing}
            loading={isProbing}
            onClick={() => void handleTestConnection()}
            data-testid="mcp-wizard-test"
          >
            Test connection
          </Button>
        )}
        {probeSummary && <p className={styles.text}>{probeSummary}</p>}
      </div>

      <Button
        type="button"
        variant="primary"
        disabled={!canSubmit}
        onClick={handleSubmit}
        data-testid="mcp-wizard-submit"
      >
        Continue with setup
      </Button>
    </Surface>
  );
};
