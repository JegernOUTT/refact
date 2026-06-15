import * as RadioGroupPrimitive from "@radix-ui/react-radio-group";
import { FC, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useGetAutoNameMutation } from "../../../services/refact/mcpMarketplace";
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

function detectTransport(input: string): "stdio" | "http" | "sse" {
  const trimmed = input.trim();
  if (trimmed.startsWith("http://") || trimmed.startsWith("https://")) {
    return "http";
  }
  return "stdio";
}

function getConfigPrefix(transport: "stdio" | "http" | "sse"): string {
  if (transport === "http") return "mcp_http_";
  if (transport === "sse") return "mcp_sse_";
  return "mcp_stdio_";
}

export const MCPSetupWizard: FC<MCPSetupWizardProps> = ({
  integration,
  onSubmit,
}) => {
  const [input, setInput] = useState("");
  const [suggestedName, setSuggestedName] = useState("");
  const [nameError, setNameError] = useState("");
  const [transport, setTransport] = useState<"stdio" | "http" | "sse">("stdio");
  const [useSSE, setUseSSE] = useState(false);
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [selectedConfigPath, setSelectedConfigPath] = useState(
    integration.integr_config_path[0] ?? "",
  );

  const [getAutoName] = useGetAutoNameMutation();
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

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
  const configPrefix = getConfigPrefix(effectiveTransport);

  const transportLabel =
    effectiveTransport === "stdio"
      ? "Local server (stdio)"
      : effectiveTransport === "sse"
        ? "Remote server (SSE)"
        : "Remote server (HTTP)";

  const handleInputChange = useCallback(
    (value: string) => {
      setInput(value);
      const detected = detectTransport(value);
      setTransport(detected);
      if (detected !== "stdio") {
        setUseSSE(false);
      }

      if (debounceRef.current) {
        clearTimeout(debounceRef.current);
      }

      if (!value.trim()) {
        setSuggestedName("");
        return;
      }

      debounceRef.current = setTimeout(() => {
        void getAutoName({ input: value.trim() })
          .unwrap()
          .then((result) => {
            setSuggestedName(result.suggested_name);
            setTransport(result.transport);
            if (!validateSnakeCase(result.suggested_name)) {
              setNameError("The name must be in snake_case!");
            } else {
              setNameError("");
            }
          })
          .catch(() => {
            const trimmed = value.trim();
            const fallback = trimmed
              .split(/[^a-z0-9]+/i)
              .filter(Boolean)
              .map((s) => s.toLowerCase())
              .join("_")
              .replace(/^_+|_+$/g, "")
              .slice(0, 40);
            setSuggestedName(fallback || "mcp_server");
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

  const canSubmit = !!input.trim() && !!suggestedName && !nameError;

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
