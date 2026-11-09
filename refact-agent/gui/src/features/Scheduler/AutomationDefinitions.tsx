import { useEffect, useState } from "react";
import {
  Button,
  Field,
  FieldError,
  FieldTextarea,
  FieldText,
  LoadingState,
} from "../../components/ui";
import { humanizeIdentifier } from "../../utils/displayNames";
import {
  schedulerErrorMessage,
  useGetAutomationsQuery,
  useSaveAutomationMutation,
  useControlAutomationMutation,
} from "../../services/refact/schedulerApi";
import {
  type AutomationDefinition,
  type AutomationEntry,
  type AutomationView,
  parseDefinitionDraft,
  supportsDefinition,
} from "./automationTypes";
import styles from "./Scheduler.module.css";

function initialDefinition(view: AutomationView): AutomationDefinition {
  return {
    schema_version: 1,
    id: "new-automation",
    revision: 1,
    owner: { kind: "project" },
    description: "New automation",
    enabled: false,
    trigger:
      view === "channel_event"
        ? { kind: view, channel: "input", filter: null }
        : view === "schedule"
          ? { kind: view, schedule: { kind: "interval", every_ms: 60000 } }
          : { kind: view },
    action:
      view === "channel_event"
        ? {
            kind: "transform",
            input_channel: "input",
            output_channel: "output",
            select_fields: [],
            max_output_bytes: 65536,
          }
        : {
            kind: "collect",
            driver: {
              type: "http_poll",
              url: "https://example.com/data",
              every_ms: 60000,
            },
          },
    deliveries:
      view === "channel_event"
        ? [{ id: "output", channel: "output", credential_ref: null }]
        : [],
    lifecycle:
      view === "continuous"
        ? {
            kind: "background",
            resume_after_restart: true,
            expires_at_ms: null,
          }
        : { kind: "on_demand" },
    policy: {
      idempotent: true,
      max_pending_events: 1024,
      max_event_bytes: 262144,
    },
  };
}

function observation(value: unknown): string {
  if (typeof value === "string") return humanizeIdentifier(value);
  if (typeof value === "number" && Number.isFinite(value)) return String(value);
  if (typeof value === "boolean") return value ? "Yes" : "No";
  return "Not observed";
}

function StatusDimensions({ entry }: { entry: AutomationEntry }) {
  return (
    <dl className={styles.jobMeta}>
      <div className={styles.jobMetaItem}>
        <dt>Desired state</dt>
        <dd>{entry.definition.enabled ? "Enabled" : "Disabled"}</dd>
      </div>
      <div className={styles.jobMetaItem}>
        <dt>Authorization</dt>
        <dd>
          {entry.status.authorization
            ? humanizeIdentifier(entry.status.authorization)
            : "Not observed"}
        </dd>
      </div>
      <div className={styles.jobMetaItem}>
        <dt>Execution health</dt>
        <dd>
          {entry.status.execution
            ? observation(entry.status.execution.state)
            : "Not observed"}
        </dd>
      </div>
      <div className={styles.jobMetaItem}>
        <dt>Data freshness / age</dt>
        <dd>
          {entry.status.freshness
            ? `${observation(
                entry.status.freshness.data_age_ms,
              )} ms · last data ${
                typeof entry.status.freshness.last_data_at_ms === "number"
                  ? new Date(
                      entry.status.freshness.last_data_at_ms,
                    ).toLocaleString()
                  : "not observed"
              } · error ${observation(entry.status.freshness.error_class)}`
            : "Not observed"}
        </dd>
      </div>
      <div className={styles.jobMetaItem}>
        <dt>Delivery lag / error</dt>
        <dd>
          {entry.status.delivery
            ? `Lag ${observation(
                entry.status.delivery.lag_ms,
              )} ms · error ${observation(entry.status.delivery.error)}`
            : "Not observed"}
        </dd>
      </div>
    </dl>
  );
}

export function AutomationDefinitions({
  view,
  onLockChange,
}: {
  view: AutomationView;
  onLockChange?: (locked: boolean) => void;
}) {
  const query = useGetAutomationsQuery(undefined, {
    pollingInterval: 15000,
    skipPollingIfUnfocused: true,
  });
  const [save, saveState] = useSaveAutomationMutation();
  const [control, controlState] = useControlAutomationMutation();
  const [editor, setEditor] = useState<{
    id?: string;
    revision?: number;
    catalogRevision: number;
    text: string;
  } | null>(null);
  const [sourceLocation, setSourceLocation] = useState("");
  const [error, setError] = useState<string | null>(null);
  const busy =
    saveState.isLoading || controlState.isLoading || query.isFetching;
  useEffect(() => {
    onLockChange?.(
      editor !== null || saveState.isLoading || controlState.isLoading,
    );
  }, [editor, saveState.isLoading, controlState.isLoading, onLockChange]);
  const entries =
    query.data?.definitions.filter(
      (entry) =>
        entry.definition.trigger.kind === view ||
        !["schedule", "continuous", "channel_event"].includes(
          entry.definition.trigger.kind,
        ),
    ) ?? [];
  const edit = (definition?: AutomationDefinition) => {
    if (!query.data) return;
    setError(null);
    setEditor({
      id: definition?.id,
      revision: definition?.revision,
      catalogRevision: query.data.catalog_revision,
      text: JSON.stringify(
        definition
          ? { ...definition, revision: definition.revision + 1 }
          : initialDefinition(view),
        null,
        2,
      ),
    });
  };
  const sourceTemplate = (
    type: "http_poll" | "sse" | "websocket" | "file_tail",
  ) => {
    if (!editor) return;
    try {
      const definition = parseDefinitionDraft(editor.text);
      const location = sourceLocation.trim();
      if (!location)
        throw new Error(
          "Enter a source URL or project-relative file path first.",
        );
      if (type !== "file_tail") {
        const url = new URL(location);
        const protocols =
          type === "websocket" ? ["ws:", "wss:"] : ["http:", "https:"];
        if (
          !protocols.includes(url.protocol) ||
          url.username ||
          url.password ||
          url.search ||
          url.hash
        )
          throw new Error(
            "Use a supported URL without credentials, query parameters or fragment.",
          );
      } else if (
        location.startsWith("/") ||
        location.includes("..") ||
        location.includes("\\")
      )
        throw new Error("Use a project-relative file path without traversal.");
      const driver =
        type === "file_tail"
          ? { type, path: location, every_ms: 1000 }
          : type === "http_poll"
            ? { type, url: location, every_ms: 60000 }
            : { type, url: location };
      setEditor({
        ...editor,
        text: JSON.stringify(
          {
            ...definition,
            trigger: { kind: "continuous" },
            lifecycle: {
              kind: "background",
              resume_after_restart: true,
              expires_at_ms: null,
            },
            action: { kind: "collect", driver },
          },
          null,
          2,
        ),
      });
      setError(null);
    } catch (failure) {
      setError(
        failure instanceof Error ? failure.message : "Invalid source template",
      );
    }
  };
  const submit = async () => {
    if (!editor) return;
    try {
      const definition = parseDefinitionDraft(editor.text);
      if (
        editor.id &&
        (definition.id !== editor.id ||
          definition.revision <= (editor.revision ?? 0))
      )
        throw new Error(
          "Keep the same ID and increase the definition revision.",
        );
      await save({
        ...(editor.id
          ? { id: editor.id, expected_revision: editor.revision }
          : {}),
        expected_catalog_revision: editor.catalogRevision,
        definition,
      }).unwrap();
      setEditor(null);
      setError(null);
    } catch (failure) {
      setError(
        failure instanceof Error
          ? failure.message
          : schedulerErrorMessage(failure),
      );
    }
  };
  const operate = async (
    entry: AutomationEntry,
    operation: "enable" | "disable" | "run_now" | "revoke" | "delete",
  ) => {
    if (!query.data) return;
    setError(null);
    try {
      await control({
        id: entry.definition.id,
        expected_revision: entry.definition.revision,
        expected_catalog_revision: query.data.catalog_revision,
        operation,
      }).unwrap();
    } catch (failure) {
      setError(schedulerErrorMessage(failure));
    }
  };
  return (
    <section className={styles.panel} aria-label="Automation definitions">
      <p className={styles.sectionHint}>
        Project definitions are desired configuration, not permission to
        execute. Missing observations do not imply success. App sources are
        managed through their manifest.
      </p>
      <div className={styles.formActions}>
        <Button
          variant="soft"
          disabled={!query.data || busy || editor !== null}
          onClick={() => edit()}
        >
          New definition
        </Button>
        <Button
          variant="soft"
          disabled={busy}
          onClick={() => void query.refetch()}
        >
          Refresh definitions
        </Button>
      </div>
      {query.isLoading ? (
        <LoadingState label="Loading Automation definitions" />
      ) : null}
      {query.error ? (
        <FieldError>{schedulerErrorMessage(query.error)}</FieldError>
      ) : null}
      {error ? (
        <FieldError>
          {error} Refresh and reopen the editor if revisions conflict; your
          draft has been retained.
        </FieldError>
      ) : null}
      {editor ? (
        <form
          className={styles.form}
          onSubmit={(event) => {
            event.preventDefault();
            void submit();
          }}
        >
          <Field
            label="Source URL or file path"
            helper="Optional collector starter. HTTP polls every 60 seconds; file tail checks every second. Editing a starter does not grant network or file access."
          >
            <FieldText
              aria-label="Source URL or file path"
              value={sourceLocation}
              onChange={setSourceLocation}
            />
          </Field>
          <div className={styles.formActions}>
            {(["http_poll", "sse", "websocket", "file_tail"] as const).map(
              (type) => (
                <Button
                  type="button"
                  key={type}
                  variant="soft"
                  disabled={busy}
                  onClick={() => sourceTemplate(type)}
                >
                  Use {humanizeIdentifier(type)}
                </Button>
              ),
            )}
          </div>
          <Field
            label="Definition JSON"
            helper="Advanced editor · 64 KiB maximum · complete schema v1 definition. Server validates driver, trigger and transform configuration. No source evaluation or inline secrets. Use credential_ref metadata only."
          >
            <FieldTextarea
              className={styles.monoField}
              aria-label="Definition JSON"
              value={editor.text}
              maxLength={65536}
              rows={16}
              onChange={(text) => setEditor({ ...editor, text })}
            />
          </Field>
          <p className={styles.sectionHint}>
            Saving never approves execution. For Needs approval, ask in chat to
            inspect this definition with the schedule tool and use the trusted
            human confirmation flow. Do not paste credentials into chat or this
            editor.
          </p>
          <div className={styles.formActions}>
            <Button type="submit" disabled={busy} loading={saveState.isLoading}>
              Save definition
            </Button>
            <Button
              type="button"
              variant="soft"
              disabled={saveState.isLoading}
              onClick={() => setEditor(null)}
            >
              Cancel definition
            </Button>
          </div>
        </form>
      ) : null}
      {query.data && entries.length === 0 ? (
        <p role="status">No definitions in this view.</p>
      ) : null}
      <div className={styles.jobList}>
        {entries.map((entry) => {
          const definition = entry.definition;
          const supported = supportsDefinition(definition);
          return (
            <article className={styles.jobCard} key={definition.id}>
              <h3 className={styles.jobTitle}>
                {definition.description || definition.id}
              </h3>
              <p className={styles.jobCron}>
                {definition.id} · revision {definition.revision} ·{" "}
                {humanizeIdentifier(definition.trigger.kind)} /{" "}
                {humanizeIdentifier(definition.action.kind)}
              </p>
              <StatusDimensions entry={entry} />
              {entry.project_wake ? (
                <p className={styles.sectionHint}>
                  Project aggregate (not this definition):{" "}
                  {entry.project_wake.pending_delivery} pending deliveries ·{" "}
                  {entry.project_wake.recovery} awaiting recovery · background{" "}
                  {entry.project_wake.background_active ? "active" : "inactive"}
                </p>
              ) : null}
              {entry.status.authorization === "needs_approval" ? (
                <p className={styles.sectionHint}>
                  Needs approval: inspect this definition with the schedule tool
                  in chat and use trusted human confirmation. Enabling does not
                  authorize execution.
                </p>
              ) : null}
              {!supported ? (
                <p className={styles.sectionHint}>
                  Read-only: unsupported definition or App-owned source. Manage
                  App sources through the guarded App manifest flow.
                </p>
              ) : null}
              <details>
                <summary>Definition and source status</summary>
                <pre className={styles.jsonPreview}>
                  {JSON.stringify(
                    { definition, status: entry.status },
                    null,
                    2,
                  )}
                </pre>
              </details>
              <div className={styles.formActions}>
                {(["enable", "disable", "run_now", "revoke"] as const)
                  .filter(
                    (operation) =>
                      entry.capabilities?.desired_control.includes(operation),
                  )
                  .map((operation) => (
                    <Button
                      key={operation}
                      variant="soft"
                      disabled={busy || !supported || editor !== null}
                      onClick={() => {
                        if (
                          operation !== "revoke" ||
                          window.confirm(
                            `Revoke authorization for ${definition.id}?`,
                          )
                        )
                          void operate(entry, operation);
                      }}
                    >
                      {humanizeIdentifier(operation)}
                    </Button>
                  ))}
                <Button
                  variant="soft"
                  disabled={busy || !supported || editor !== null}
                  onClick={() => edit(definition)}
                >
                  Edit definition
                </Button>
                <Button
                  variant="danger"
                  disabled={busy || !supported || editor !== null}
                  onClick={() => {
                    if (window.confirm(`Delete automation ${definition.id}?`))
                      void operate(entry, "delete");
                  }}
                >
                  Delete definition
                </Button>
              </div>
            </article>
          );
        })}
      </div>
    </section>
  );
}
