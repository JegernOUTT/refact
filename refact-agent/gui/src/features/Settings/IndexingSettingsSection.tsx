import React, { useEffect, useMemo, useRef, useState } from "react";

import {
  Button,
  SegmentedControl,
  SettingItem,
  SettingsListEditor,
  StatusBadge,
} from "../../components/ui";
import type { SettingsListEditorItem } from "../../components/ui";
import { useIsDocumentVisible } from "../../hooks/useIsDocumentVisible";
import {
  useGetIndexingSettingsQuery,
  useSaveIndexingSettingsMutation,
} from "../../services/refact/indexingSettings";
import type {
  IndexingSettingsConfig,
  IndexingSettingsScope,
} from "../../services/refact/indexingSettings";
import { useGetRagStatusQuery } from "../../services/refact/ragStatus";
import { SettingsGroup, SettingsSection } from "./SettingsSection";
import styles from "./IndexingSettingsSection.module.css";

function errorMessage(error: unknown): string {
  if (!error || typeof error !== "object") return "The request failed.";
  if ("error" in error && typeof error.error === "string") return error.error;
  if ("data" in error) {
    const data = error.data;
    if (typeof data === "string") return data;
    if (data && typeof data === "object" && "detail" in data) {
      const detail = data.detail;
      if (typeof detail === "string") return detail;
    }
  }
  return "The request failed.";
}

function statusTone(state: string): "success" | "danger" | "accent" | "muted" {
  if (state === "working" || state === "done") return "success";
  if (state === "error") return "danger";
  if (state === "indexing" || state === "parsing" || state === "starting") {
    return "accent";
  }
  return "muted";
}

function IndexingStatus() {
  const visible = useIsDocumentVisible();
  const { data, error, isLoading } = useGetRagStatusQuery(undefined, {
    pollingInterval: visible ? 3000 : 0,
  });
  const vecErrors = data?.vecdb
    ? Object.entries(data.vecdb.vecdb_errors).filter(([, count]) => count > 0)
    : [];
  const codeGraphError = data?.codegraph?.error ?? data?.codegraph_error;

  if (isLoading) {
    return <p className={styles.muted}>Loading live indexing status…</p>;
  }
  if (error !== undefined || !data) {
    return (
      <p className={styles.muted} role="status">
        Live indexing status is unavailable. Settings can still be edited.
      </p>
    );
  }

  const codeGraphState = data.codegraph?.state ?? data.codegraph_alive;
  const vecDbState = data.vecdb?.state ?? data.vecdb_alive;

  return (
    <div className={styles.statusGrid}>
      <div className={styles.statusCard}>
        <div className={styles.statusHeading}>
          <strong>CodeGraph</strong>
          <StatusBadge
            status={codeGraphState || "idle"}
            label={codeGraphState || "Unavailable"}
            tone={statusTone(codeGraphState)}
          />
        </div>
        <p className={styles.muted}>
          {data.codegraph
            ? `${data.codegraph.queued} file${
                data.codegraph.queued === 1 ? "" : "s"
              } queued`
            : "No CodeGraph details reported."}
        </p>
        {codeGraphError ? (
          <p className={styles.error} role="alert">
            {codeGraphError}
          </p>
        ) : null}
      </div>
      <div className={styles.statusCard}>
        <div className={styles.statusHeading}>
          <strong>VecDB</strong>
          <StatusBadge
            status={vecDbState || "idle"}
            label={vecDbState || "Unavailable"}
            tone={statusTone(vecDbState)}
          />
        </div>
        <p className={styles.muted}>
          {data.vecdb
            ? `${data.vecdb.files_unprocessed} of ${data.vecdb.files_total} files remaining`
            : "No VecDB details reported."}
        </p>
        {data.vecdb?.vecdb_max_files_hit ? (
          <p className={styles.warning}>
            The runtime maximum file limit was reached.
          </p>
        ) : null}
        {data.vec_db_error ? (
          <p className={styles.error} role="alert">
            {data.vec_db_error}
          </p>
        ) : null}
        {vecErrors.length > 0 ? (
          <ul className={styles.errorList} aria-label="VecDB errors">
            {vecErrors.map(([message, count]) => (
              <li key={message}>
                {message} ({count})
              </li>
            ))}
          </ul>
        ) : null}
      </div>
    </div>
  );
}

export const IndexingSettingsSection: React.FC = () => {
  const nextId = useRef(0);
  const [scope, setScope] = useState<IndexingSettingsScope>("global");
  const [blocklist, setBlocklist] = useState<SettingsListEditorItem[]>([]);
  const [directories, setDirectories] = useState<SettingsListEditorItem[]>([]);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const { data, error, isFetching } = useGetIndexingSettingsQuery(scope);
  const [saveSettings, saveState] = useSaveIndexingSettingsMutation();

  useEffect(() => {
    if (!data) return;
    if (scope === "project" && !data.project_available) {
      setScope("global");
      return;
    }
    setBlocklist(
      data.config.blocklist.map((value) => ({
        id: `block-${nextId.current++}`,
        value,
      })),
    );
    setDirectories(
      data.config.additional_indexing_dirs.map((value) => ({
        id: `dir-${nextId.current++}`,
        value,
      })),
    );
    setSaveError(null);
    setSaved(false);
  }, [data, scope]);

  const projectAvailable = data?.project_available ?? true;
  const cleanedConfig = useMemo<IndexingSettingsConfig>(
    () => ({
      blocklist: blocklist.map(({ value }) => value.trim()).filter(Boolean),
      additional_indexing_dirs: directories
        .map(({ value }) => value.trim())
        .filter(Boolean),
    }),
    [blocklist, directories],
  );

  const changeItem = (
    setter: React.Dispatch<React.SetStateAction<SettingsListEditorItem[]>>,
    id: string,
    value: string,
  ) => {
    setter((items) =>
      items.map((item) => (item.id === id ? { ...item, value } : item)),
    );
    setSaved(false);
  };

  const removeItem = (
    setter: React.Dispatch<React.SetStateAction<SettingsListEditorItem[]>>,
    id: string,
  ) => {
    setter((items) => items.filter((item) => item.id !== id));
    setSaved(false);
  };

  const addItem = (
    setter: React.Dispatch<React.SetStateAction<SettingsListEditorItem[]>>,
    prefix: string,
  ) => {
    setter((items) => [
      ...items,
      { id: `${prefix}-${nextId.current++}`, value: "" },
    ]);
    setSaved(false);
  };

  const handleSave = async () => {
    setSaveError(null);
    if (scope === "project" && !projectAvailable) {
      setSaveError("Open a project before saving project indexing settings.");
      return;
    }
    try {
      await saveSettings({ scope, config: cleanedConfig }).unwrap();
      setSaved(true);
    } catch (requestError) {
      setSaved(false);
      setSaveError(errorMessage(requestError));
    }
  };

  return (
    <SettingsSection
      title="Indexing"
      description="Control which files and directories are included in project context."
      actions={
        <Button
          variant="primary"
          loading={saveState.isLoading}
          disabled={isFetching || Boolean(error)}
          onClick={() => void handleSave()}
        >
          Save
        </Button>
      }
    >
      <SettingsGroup title="Live status">
        <IndexingStatus />
      </SettingsGroup>

      <SettingsGroup title="Configuration scope">
        <SettingItem
          className="rf-enter"
          title="Scope"
          description={
            scope === "global"
              ? "Global settings apply across projects."
              : "Project settings apply only to the currently open project."
          }
          control={
            <SegmentedControl
              name="indexing-scope"
              value={scope}
              options={[
                { value: "global", label: "Global" },
                {
                  value: "project",
                  label: "This project",
                  disabled: !projectAvailable,
                },
              ]}
              onValueChange={(value) => {
                setScope(value as IndexingSettingsScope);
                setSaveError(null);
              }}
            />
          }
        />
        {!projectAvailable ? (
          <p className={styles.notice} role="status">
            No project is currently available. Open a project to edit its
            indexing settings.
          </p>
        ) : null}
        <SettingItem
          className="rf-enter"
          title="Configuration file"
          description={
            data?.path ?? (isFetching ? "Loading…" : "Path unavailable")
          }
        />
        {error ? (
          <p className={styles.error} role="alert">
            Could not load indexing settings: {errorMessage(error)}
          </p>
        ) : null}
      </SettingsGroup>

      <SettingsGroup title="Included files">
        <SettingItem
          className="rf-enter"
          layout="stack"
          title="Blocklist globs"
          description="Files matching these globs are skipped. Enter one glob per row."
          control={
            <SettingsListEditor
              items={blocklist}
              disabled={isFetching || saveState.isLoading}
              addLabel="Add glob"
              placeholder="*/generated/*"
              monospace
              emptyLabel="No blocklist globs configured."
              itemAriaLabel={(_item, index) =>
                `Remove blocklist glob ${index + 1}`
              }
              onAdd={() => addItem(setBlocklist, "block")}
              onChange={(id, value) => changeItem(setBlocklist, id, value)}
              onRemove={(id) => removeItem(setBlocklist, id)}
            />
          }
        />
        <SettingItem
          className="rf-enter"
          layout="stack"
          title="Additional indexing directories"
          description="Include directories outside normal project roots, such as checked-out libraries. Enter one path per row; privacy rules still apply when content is shared."
          control={
            <SettingsListEditor
              items={directories}
              disabled={isFetching || saveState.isLoading}
              addLabel="Add directory"
              placeholder="/absolute/path/to/library"
              monospace
              emptyLabel="No additional directories configured."
              itemAriaLabel={(_item, index) => `Remove directory ${index + 1}`}
              onAdd={() => addItem(setDirectories, "dir")}
              onChange={(id, value) => changeItem(setDirectories, id, value)}
              onRemove={(id) => removeItem(setDirectories, id)}
            />
          }
        />
        <p className={styles.note}>
          AST and VecDB limits are managed by the host/runtime and are not
          configured here.
        </p>
        {saveError ? (
          <p className={styles.error} role="alert">
            Could not save indexing settings: {saveError}
          </p>
        ) : null}
        {saved ? (
          <p className={styles.success} role="status">
            Indexing settings saved.
          </p>
        ) : null}
      </SettingsGroup>
    </SettingsSection>
  );
};
