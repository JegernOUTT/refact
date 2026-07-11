import { useState } from "react";
import { Upload } from "lucide-react";

import {
  Button,
  Dialog,
  FieldSwitch,
  FieldTextarea,
  Flex,
  Text,
} from "../../ui";
import {
  useImportMcpConfigMutation,
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
  const [importMcpConfig, { isLoading }] = useImportMcpConfigMutation();

  const reset = () => {
    setJsonText("");
    setOverwriteExisting(false);
    setParseError(false);
    setResult(null);
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

    try {
      const response = await importMcpConfig(body).unwrap();
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
              <FieldTextarea
                aria-label="MCP servers JSON"
                className={styles.textarea}
                rows={9}
                value={jsonText}
                onChange={setJsonText}
                placeholder={'{"mcpServers":{"github":{"command":"npx"}}}'}
              />
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
