import { useMemo, useRef, useState } from "react";
import { FolderGit2, Upload } from "lucide-react";

import {
  Button,
  Dialog,
  FieldSwitch,
  FieldText,
  FieldTextarea,
  Flex,
  Text,
} from "../../ui";
import {
  useImportMcpConfigMutation,
  useGetProjectMcpConfigQuery,
  type McpImportResponse,
} from "../../../services/refact/mcpMarketplace";
import styles from "./MCPImportDialog.module.css";

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isBareMcpServersMap(value: unknown): value is Record<string, unknown> {
  if (!isRecord(value) || "mcpServers" in value || "bundle" in value) {
    return false;
  }

  const entries = Object.values(value);
  return (
    entries.length > 0 &&
    entries.every(
      (entry) => isRecord(entry) && ("command" in entry || "url" in entry),
    )
  );
}

function getImportBody(
  parsed: unknown,
  overwriteExisting: boolean,
): Record<string, unknown> {
  const body = isBareMcpServersMap(parsed)
    ? { mcpServers: parsed }
    : isRecord(parsed)
      ? { ...parsed }
      : { value: parsed };

  return { ...body, overwrite_existing: overwriteExisting };
}

const REDACTED = "<REDACTED>";

type SecretField = { configName: string; fieldPath: string };

/** Finds `<REDACTED>` values in a Refact export bundle so the user can fill
 *  them in before importing (the engine re-injects them via `secrets`). */
function collectRedactedFields(parsed: unknown): SecretField[] {
  if (!isRecord(parsed)) return [];
  const bundle = isRecord(parsed.bundle) ? parsed.bundle : parsed;
  const servers = bundle.servers;
  if (!Array.isArray(servers)) return [];
  const fields: SecretField[] = [];
  for (const server of servers) {
    if (!isRecord(server) || typeof server.config_name !== "string") continue;
    const config = server.config;
    if (!isRecord(config)) continue;
    const walk = (value: unknown, path: string) => {
      if (value === REDACTED) {
        fields.push({
          configName: server.config_name as string,
          fieldPath: path,
        });
        return;
      }
      if (isRecord(value)) {
        for (const [key, child] of Object.entries(value)) {
          walk(child, path ? `${path}.${key}` : key);
        }
      }
    };
    for (const [key, value] of Object.entries(config)) {
      walk(value, key);
    }
  }
  return fields;
}

function entryLabel(
  entry: McpImportResponse["imported"][number],
  fallback: string,
): string {
  return entry.config_name ?? entry.config_path ?? fallback;
}

function ResultSummary({ result }: { result: McpImportResponse }) {
  return (
    <div className={styles.results} aria-live="polite">
      <ResultGroup
        className={styles.successText}
        label="Imported"
        count={result.imported.length}
        items={result.imported.map((entry, index) =>
          entryLabel(entry, `Config ${index + 1}`),
        )}
      />
      <ResultGroup
        className={styles.mutedText}
        label="Skipped"
        count={result.skipped.length}
        items={result.skipped.map((entry, index) => {
          const name = entryLabel(entry, `Config ${index + 1}`);
          return entry.reason ? `${name} (${entry.reason})` : name;
        })}
      />
      <ResultGroup
        className={styles.errorText}
        label="Errors"
        count={result.errors.length}
        items={result.errors.map((entry, index) => {
          const name = entryLabel(entry, `Config ${index + 1}`);
          return entry.error ? `${name} (${entry.error})` : name;
        })}
      />
    </div>
  );
}

function ResultGroup({
  className,
  count,
  items,
  label,
}: {
  className: string;
  count: number;
  items: string[];
  label: string;
}) {
  return (
    <div className={styles.resultGroup}>
      <Text as="div" size="2" weight="medium" className={className}>
        {label}: {count}
      </Text>
      {items.length > 0 && (
        <ul className={styles.resultList}>
          {items.map((item) => (
            <li key={item}>
              <Text size="1" className={className}>
                {item}
              </Text>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

export function MCPImportDialog() {
  const [open, setOpen] = useState(false);
  const [jsonText, setJsonText] = useState("");
  const [overwriteExisting, setOverwriteExisting] = useState(false);
  const [parseError, setParseError] = useState(false);
  const [result, setResult] = useState<McpImportResponse | null>(null);
  const [requestError, setRequestError] = useState(false);
  const [secretValues, setSecretValues] = useState<
    Record<string, string | undefined>
  >({});
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const [importMcpConfig, { isLoading }] = useImportMcpConfigMutation();
  const { data: projectConfigData } = useGetProjectMcpConfigQuery(undefined, {
    skip: !open,
  });

  const projectServerCount = useMemo(
    () =>
      (projectConfigData?.project_configs ?? [])
        .filter((entry) => !entry.error)
        .reduce((sum, entry) => sum + (entry.server_count ?? 0), 0),
    [projectConfigData],
  );

  const redactedFields = useMemo(() => {
    try {
      return collectRedactedFields(JSON.parse(jsonText));
    } catch {
      return [];
    }
  }, [jsonText]);

  const readFileIntoTextarea = (file: File) => {
    const reader = new FileReader();
    reader.onload = () => {
      if (typeof reader.result === "string") {
        setJsonText(reader.result);
        setParseError(false);
        setResult(null);
      }
    };
    reader.readAsText(file);
  };

  const reset = () => {
    setJsonText("");
    setOverwriteExisting(false);
    setParseError(false);
    setResult(null);
    setSecretValues({});
  };

  const handleOpenChange = (nextOpen: boolean) => {
    setOpen(nextOpen);
    if (!nextOpen) {
      reset();
    }
  };

  const closeAndReset = () => {
    setOpen(false);
    reset();
  };

  const handleImport = async () => {
    setParseError(false);
    setRequestError(false);
    setResult(null);

    let body: Record<string, unknown>;
    try {
      const parsed = JSON.parse(jsonText) as unknown;
      body = getImportBody(parsed, overwriteExisting);
    } catch {
      setParseError(true);
      return;
    }

    const secretsMap = new Map<string, Record<string, string>>();
    for (const field of redactedFields) {
      const value = secretValues[`${field.configName}\u0000${field.fieldPath}`];
      if (!value?.trim()) continue;
      const bucket = secretsMap.get(field.configName) ?? {};
      bucket[field.fieldPath] = value;
      secretsMap.set(field.configName, bucket);
    }
    if (secretsMap.size > 0) {
      body.secrets = Object.fromEntries(secretsMap);
    }

    try {
      const response = await importMcpConfig(body).unwrap();
      setResult(response);
    } catch {
      setRequestError(true);
    }
  };

  const handleImportFromProject = async () => {
    setParseError(false);
    setRequestError(false);
    setResult(null);
    try {
      const response = await importMcpConfig({
        from_project: true,
        overwrite_existing: overwriteExisting,
      }).unwrap();
      setResult(response);
    } catch {
      setRequestError(true);
    }
  };

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <Dialog.Trigger asChild>
        <Button variant="ghost" size="sm" leftIcon={Upload}>
          Import
        </Button>
      </Dialog.Trigger>
      <Dialog.Content>
        <Dialog.Title>Import MCP servers</Dialog.Title>
        <Dialog.Description>
          Paste a Refact export bundle or a Claude Desktop / VS Code style
          mcpServers JSON.
        </Dialog.Description>

        <Flex direction="column" gap="3">
          {!result && (
            <>
              <div
                onDragOver={(event) => event.preventDefault()}
                onDrop={(event) => {
                  event.preventDefault();
                  const file = event.dataTransfer.files.item(0);
                  if (file) readFileIntoTextarea(file);
                }}
              >
                <FieldTextarea
                  aria-label="MCP servers JSON"
                  className={styles.textarea}
                  rows={9}
                  value={jsonText}
                  onChange={setJsonText}
                  placeholder={'{"mcpServers":{"github":{"command":"npx"}}}'}
                />
              </div>
              <Flex align="center" gap="2" wrap="wrap">
                <input
                  ref={fileInputRef}
                  type="file"
                  accept=".json,application/json"
                  hidden
                  aria-label="Choose MCP config file"
                  onChange={(event) => {
                    const file = event.target.files?.item(0);
                    if (file) readFileIntoTextarea(file);
                    event.target.value = "";
                  }}
                />
                <Button
                  variant="ghost"
                  size="sm"
                  leftIcon={Upload}
                  onClick={() => fileInputRef.current?.click()}
                >
                  Load file…
                </Button>
                {projectServerCount > 0 && (
                  <Button
                    variant="ghost"
                    size="sm"
                    leftIcon={FolderGit2}
                    onClick={() => void handleImportFromProject()}
                    disabled={isLoading}
                  >
                    Import {projectServerCount} from project config
                  </Button>
                )}
              </Flex>
              {redactedFields.length > 0 && (
                <Flex direction="column" gap="2">
                  <Text size="2" weight="medium">
                    Fill in redacted secrets
                  </Text>
                  {redactedFields.map((field) => {
                    const key = `${field.configName}\u0000${field.fieldPath}`;
                    return (
                      <Flex key={key} align="center" gap="2">
                        <Text size="1" className={styles.mutedText}>
                          {field.configName} · {field.fieldPath}
                        </Text>
                        <FieldText
                          type="password"
                          aria-label={`Secret for ${field.configName} ${field.fieldPath}`}
                          value={secretValues[key] ?? ""}
                          onChange={(nextValue) =>
                            setSecretValues((prev) => ({
                              ...prev,
                              [key]: nextValue,
                            }))
                          }
                        />
                      </Flex>
                    );
                  })}
                </Flex>
              )}
              {parseError && (
                <Text as="p" size="2" className={styles.errorText}>
                  Invalid JSON
                </Text>
              )}
              {requestError && (
                <Text as="p" size="2" className={styles.errorText}>
                  Import failed. Check that the engine is reachable and the JSON
                  shape is supported.
                </Text>
              )}
              <Flex align="center" gap="2">
                <FieldSwitch
                  aria-label="Overwrite existing configs"
                  checked={overwriteExisting}
                  onChange={setOverwriteExisting}
                />
                <Text size="2">Overwrite existing configs</Text>
              </Flex>
            </>
          )}

          {result && <ResultSummary result={result} />}

          <Flex justify="end" gap="2" wrap="wrap">
            {result ? (
              <Button variant="primary" onClick={closeAndReset}>
                Done
              </Button>
            ) : (
              <>
                <Button variant="ghost" onClick={closeAndReset}>
                  Cancel
                </Button>
                <Button
                  variant="primary"
                  onClick={() => void handleImport()}
                  disabled={isLoading}
                  loading={isLoading}
                >
                  Import
                </Button>
              </>
            )}
          </Flex>
        </Flex>
      </Dialog.Content>
    </Dialog>
  );
}
