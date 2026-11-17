import { useState } from "react";
import type { ReactNode } from "react";
import type { FetchBaseQueryError } from "@reduxjs/toolkit/query";
import { Download, FileSearch, ListChecks } from "lucide-react";

import {
  Badge,
  Button,
  FieldError,
  FieldSelect,
  FieldText,
  LoadingState,
  SettingItem,
  Surface,
} from "../../../components/ui";
import {
  parseRuntimeSource,
  RUNTIME_REPOSITORY,
  useGetRuntimeStatusQuery,
  useResolveRuntimeMutation,
  useSelectRuntimeMutation,
  useStageRuntimeMutation,
} from "../../../services/refact/runtimeCoordinator";
import type {
  ComponentVersions,
  ManagedIdentity,
  Observation,
  RuntimeChannel,
  RuntimeSelection,
  RuntimeStageInfo,
  RuntimeUpdatePlan,
  RuntimeStatusResponse,
} from "../../../services/refact/runtimeCoordinator";
import { SettingsGroup } from "../../Settings/SettingsSection";
import styles from "./RuntimeCoordinatorSection.module.css";

const DEFAULT_SOURCE = `https://github.com/${RUNTIME_REPOSITORY.owner}/${RUNTIME_REPOSITORY.name}`;
const AUTH_REQUIRED =
  "The daemon rejected this change: this view is not signed in as the daemon user. Use the refact runtime CLI in a terminal on the machine running the daemon instead.";
const CONFLICT_GUIDANCE =
  "Runtime status was refreshed; review it before retrying.";

type RuntimeAction = keyof RuntimeStatusResponse["actions"];
const RUNTIME_ACTIONS: readonly RuntimeAction[] = [
  "select",
  "resolve",
  "stage",
];
const PREPARE_ACTIONS: readonly RuntimeAction[] = ["resolve", "stage"];
const ACTION_LABELS: Record<RuntimeAction, string> = {
  select: "Runtime selection",
  resolve: "Update-plan resolution",
  stage: "Runtime staging",
};

const CHANNEL_OPTIONS: { value: RuntimeChannel; label: string }[] = [
  { value: "stable", label: "Stable" },
  { value: "preview", label: "Preview" },
  { value: "nightly", label: "Nightly" },
];

function isChannel(value: string): value is RuntimeChannel {
  return CHANNEL_OPTIONS.some((option) => option.value === value);
}

function backendText(queryError: FetchBaseQueryError | undefined) {
  if (queryError && "data" in queryError) {
    const data = queryError.data;
    if (typeof data === "string" && data) return data;
    if (typeof data === "object" && data !== null && "error" in data) {
      const message = (data as { error?: unknown }).error;
      if (typeof message === "string" && message) return message;
    }
  }
  return null;
}

function errorMessage(error: unknown, fallback: string): string {
  const queryError = error as FetchBaseQueryError | undefined;
  if (queryError?.status === 401) return AUTH_REQUIRED;
  const text = backendText(queryError) ?? fallback;
  return queryError?.status === 409 ? `${text}. ${CONFLICT_GUIDANCE}` : text;
}

function describeSelection(selection: RuntimeSelection): string {
  switch (selection.kind) {
    case "channel":
      return `${selection.repository.owner}/${selection.repository.name} · ${selection.channel} channel`;
    case "pinned":
      return selection.source.kind === "release"
        ? `${selection.repository.owner}/${selection.repository.name} · release ${selection.source.tag}`
        : `${selection.repository.owner}/${selection.repository.name} · Actions run ${selection.source.id} attempt ${selection.source.attempt}`;
    case "local":
      return `Local runtime ${selection.absolute_path} (unmanaged, never overwritten)`;
  }
}

function describeVersions(versions: ComponentVersions): string {
  return `runtime ${versions.runtime} · GUI ${versions.gui} · VS Code ${versions.vscode} · JetBrains ${versions.jetbrains}`;
}

function describeIdentity(identity: ManagedIdentity): string {
  return `${describeVersions(identity.component_versions)} · ${
    identity.target
  } · manifest ${identity.manifest_digest}`;
}

function ObservationRow<T>({
  title,
  observation,
  describe,
}: {
  title: string;
  observation: Observation<T> | undefined;
  describe: (value: T) => string;
}) {
  const known = observation?.state === "known";
  return (
    <div className={styles.observation}>
      <dt className={styles.term}>{title}</dt>
      <dd className={styles.detail}>
        {known ? (
          <span className={styles.value}>{describe(observation.value)}</span>
        ) : (
          <Badge size="sm" tone="muted" variant="soft">
            Unknown
          </Badge>
        )}
      </dd>
    </div>
  );
}

function RuntimeGuidance({
  response,
  source,
  channel,
  selectedManaged,
}: {
  response: RuntimeStatusResponse;
  source: string;
  channel: RuntimeChannel;
  selectedManaged: boolean;
}) {
  const { actions, status, target } = response;
  if (Object.values(actions).every((action) => action.available)) return null;
  const reasons = Object.values(actions).map((action) => action.reason);
  const cliNeeded =
    reasons.includes("auth_unconfigured") ||
    reasons.includes("unauthenticated");
  const unsupported = (action: RuntimeAction) =>
    actions[action].reason === "platform_unsupported";
  const trimmedSource = source.trim();
  const commandSource = /^[\w:/%.=-]+$/.test(trimmedSource)
    ? trimmedSource
    : `'${trimmedSource.replace(/'/g, "'\\''")}'`;
  const selectLine = parseRuntimeSource(trimmedSource, channel) ? (
    <code key="select" className={styles.value}>
      {`refact runtime select --revision ${status.selection_revision} --source ${commandSource} --channel ${channel}`}
    </code>
  ) : (
    <p key="select" className={styles.muted}>
      Enter a valid runtime source URL to see the select command.
    </p>
  );
  const prepareLines = (revision: string) =>
    PREPARE_ACTIONS.filter((action) => !unsupported(action)).map((action) => (
      <code key={action} className={styles.value}>
        {`refact runtime ${action} --revision ${revision}`}
      </code>
    ));
  let commandLines: ReactNode = null;
  if (cliNeeded && selectedManaged) {
    commandLines = (
      <>
        <p className={styles.muted}>
          {`For the current selection (revision ${status.selection_revision}):`}
        </p>
        {prepareLines(String(status.selection_revision))}
        {unsupported("select") ? null : (
          <>
            <p className={styles.muted}>
              To choose a different source instead, run select; it prints the
              new revision to use with resolve and stage:
            </p>
            {selectLine}
          </>
        )}
      </>
    );
  } else if (cliNeeded && !unsupported("select")) {
    commandLines = (
      <>
        {selectLine}
        {prepareLines("<revision printed by select>")}
      </>
    );
  }
  return (
    <div
      role="note"
      aria-label="Runtime action guidance"
      className={styles.guidance}
    >
      {commandLines === null ? null : reasons.includes("auth_unconfigured") ? (
        <p className={styles.muted}>
          Runtime changes need daemon user authentication, which is not
          configured. Turn on authentication with a username and password in the
          Daemon settings section; saving there restarts the daemon. Then run
          these commands in a terminal on the machine running the daemon:
        </p>
      ) : reasons.includes("unauthenticated") ? (
        <p className={styles.muted}>
          This view is not signed in as the daemon user, so it cannot change the
          runtime. Run these commands in a terminal on the machine running the
          daemon; the refact CLI uses the daemon credentials from its
          configuration:
        </p>
      ) : null}
      {commandLines}
      {RUNTIME_ACTIONS.filter(unsupported).map((action) => (
        <p key={action} className={styles.muted}>
          {`${ACTION_LABELS[action]} is not supported on this daemon's platform (${target}).`}
        </p>
      ))}
    </div>
  );
}

function PlanDetails({ plan }: { plan: RuntimeUpdatePlan }) {
  const tested = plan.qualification === "tested";
  return (
    <dl aria-label="Update plan" className={styles.facts}>
      <div className={styles.observation}>
        <dt className={styles.term}>Versions</dt>
        <dd className={styles.value}>{describeIdentity(plan.identity)}</dd>
      </div>
      <div className={styles.observation}>
        <dt className={styles.term}>Source</dt>
        <dd className={styles.value}>
          {`${plan.source.repository.owner}/${plan.source.repository.name} · commit ${plan.source.source_sha} · ${plan.source.workflow.path} run ${plan.source.run.id} attempt ${plan.source.run.attempt}`}
        </dd>
      </div>
      <div className={styles.observation}>
        <dt className={styles.term}>Qualification</dt>
        <dd className={styles.badges}>
          <Badge size="sm" tone={tested ? "success" : "warning"} variant="soft">
            {tested ? "Tested" : "Untested"}
          </Badge>
          <Badge size="sm" tone="muted" variant="outline">
            {`Manifest tests: ${plan.manifest_tests.replace("_", " ")}`}
          </Badge>
          <Badge
            size="sm"
            tone={plan.public_qualification_verified ? "success" : "muted"}
            variant="outline"
          >
            {plan.public_qualification_verified
              ? "Public qualification verified"
              : "No public qualification evidence"}
          </Badge>
        </dd>
      </div>
      <div className={styles.observation}>
        <dt className={styles.term}>Updated together</dt>
        <dd className={styles.value}>{plan.together.join(", ")}</dd>
      </div>
      <div className={styles.observation}>
        <dt className={styles.term}>IDE-owned, not updated</dt>
        <dd className={styles.value}>
          {plan.ide_owned_not_updated.join(", ")}
        </dd>
      </div>
    </dl>
  );
}

export function RuntimeCoordinatorSection() {
  const [channel, setChannel] = useState<RuntimeChannel>("stable");
  const [source, setSource] = useState(DEFAULT_SOURCE);
  const [plan, setPlan] = useState<RuntimeUpdatePlan | null>(null);
  const [staged, setStaged] = useState<RuntimeStageInfo | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const statusQuery = useGetRuntimeStatusQuery(undefined);
  const [selectRuntime, selectState] = useSelectRuntimeMutation();
  const [resolveRuntime, resolveState] = useResolveRuntimeMutation();
  const [stageRuntime, stageState] = useStageRuntimeMutation();

  const response = statusQuery.data;
  const status = response?.status;
  const revision = status?.selection_revision ?? 0;
  const selectedManaged =
    status?.selected.state === "known" &&
    status.selected.value.kind !== "local";
  const busy =
    selectState.isLoading || resolveState.isLoading || stageState.isLoading;
  const statusUnavailable = !response;

  function clearCandidate() {
    setPlan(null);
    setStaged(null);
  }

  async function select() {
    setActionError(null);
    const selection = parseRuntimeSource(source.trim(), channel);
    if (!selection) {
      setActionError(
        `Invalid source. Use ${DEFAULT_SOURCE}, its /releases, /releases/tag/<tag>, or /actions/runs/<id> URL.`,
      );
      return;
    }
    try {
      await selectRuntime({ expected_revision: revision, selection }).unwrap();
      clearCandidate();
    } catch (error) {
      if ((error as FetchBaseQueryError | undefined)?.status === 409) {
        clearCandidate();
      }
      setActionError(errorMessage(error, "Could not save the selection."));
    }
  }

  async function resolve() {
    setActionError(null);
    try {
      setPlan(await resolveRuntime({ expected_revision: revision }).unwrap());
    } catch (error) {
      if ((error as FetchBaseQueryError | undefined)?.status === 409) {
        clearCandidate();
      }
      setActionError(errorMessage(error, "Could not resolve the runtime."));
    }
  }

  async function stage() {
    setActionError(null);
    try {
      const result = await stageRuntime({
        expected_revision: revision,
      }).unwrap();
      setPlan(result.plan);
      setStaged(result.staged);
    } catch (error) {
      if ((error as FetchBaseQueryError | undefined)?.status === 409) {
        clearCandidate();
      }
      setActionError(errorMessage(error, "Could not stage the runtime."));
    }
  }

  return (
    <SettingsGroup title="Runtime">
      <Surface className={styles.surface} variant="glass">
        <p className={styles.copy}>
          One host runtime provides the CLI, daemon, workers and embedded GUI.
          VS Code and JetBrains plugins are updated by their IDEs.
        </p>
        {statusQuery.isLoading ? (
          <LoadingState kind="skeleton" label="Loading runtime status" />
        ) : statusQuery.isError ? (
          <FieldError>
            {errorMessage(statusQuery.error, "Runtime status is unavailable.")}
          </FieldError>
        ) : null}
        <dl aria-label="Runtime status" className={styles.facts}>
          <ObservationRow
            title="Selected"
            observation={status?.selected}
            describe={describeSelection}
          />
          {status?.installed.state !== "known" &&
          response?.running_build.managed === false ? (
            <div className={styles.observation}>
              <dt className={styles.term}>Installed</dt>
              <dd className={styles.detail}>
                <Badge size="sm" tone="muted" variant="soft">
                  Unmanaged
                </Badge>
                {response.running_build.dev_build ? (
                  <Badge size="sm" tone="muted" variant="soft">
                    Dev build
                  </Badge>
                ) : null}
                <p className={`${styles.muted} rf-wrap-anywhere`}>
                  {response.running_build.executable
                    ? `${response.running_build.executable} is not installed by the runtime coordinator.`
                    : "The running runtime is not installed by the runtime coordinator."}
                </p>
              </dd>
            </div>
          ) : (
            <ObservationRow
              title="Installed"
              observation={status?.installed}
              describe={(installed) =>
                `${describeIdentity(installed.identity)} · ${
                  installed.absolute_path
                }`
              }
            />
          )}
          <ObservationRow
            title="Staged"
            observation={status?.staged}
            describe={(identity) =>
              `${describeIdentity(identity)} (cached stage)`
            }
          />
          <ObservationRow
            title="Running"
            observation={status?.running}
            describe={(running) =>
              running.reported_versions.state === "known"
                ? `PID ${running.pid} · ${describeVersions(
                    running.reported_versions.value,
                  )}`
                : `PID ${running.pid} · runtime ${response?.running_build.version}`
            }
          />
        </dl>
        {response?.diagnostics.length ? (
          <ul aria-label="Runtime diagnostics" className={styles.diagnostics}>
            {response.diagnostics.map((line) => (
              <li key={line}>{line}</li>
            ))}
          </ul>
        ) : null}
        <SettingItem
          layout="stack"
          title="Source"
          description="Public releases of JegernOUTT/refact. Actions run URLs require GitHub authentication and are rejected by the daemon."
          control={
            <div className={styles.sourceControl}>
              <FieldSelect
                aria-label="Runtime channel"
                disabled={busy}
                value={channel}
                onChange={(value) => {
                  if (isChannel(value)) setChannel(value);
                }}
                options={CHANNEL_OPTIONS}
              />
              <FieldText
                aria-label="Runtime source URL"
                autoComplete="off"
                disabled={busy}
                spellCheck={false}
                value={source}
                onChange={setSource}
              />
              <Button
                disabled={
                  busy ||
                  statusUnavailable ||
                  !response.actions.select.available
                }
                leftIcon={ListChecks}
                loading={selectState.isLoading}
                onClick={() => void select()}
                size="sm"
                variant="soft"
              >
                Select source
              </Button>
            </div>
          }
        />
        <div className={styles.actions}>
          <Button
            disabled={
              busy || !selectedManaged || !response?.actions.resolve.available
            }
            leftIcon={FileSearch}
            loading={resolveState.isLoading}
            onClick={() => void resolve()}
            size="sm"
            variant="soft"
          >
            Resolve update plan
          </Button>
          <Button
            disabled={
              busy || !selectedManaged || !response?.actions.stage.available
            }
            leftIcon={Download}
            loading={stageState.isLoading}
            onClick={() => void stage()}
            size="sm"
            variant="primary"
          >
            Stage runtime
          </Button>
        </div>
        {response ? (
          <RuntimeGuidance
            response={response}
            source={source}
            channel={channel}
            selectedManaged={selectedManaged}
          />
        ) : null}
        {plan ? <PlanDetails plan={plan} /> : null}
        {staged ? (
          <p aria-live="polite" className={styles.notice}>
            {`Cached ${staged.target} runtime (sha256 ${staged.runtime_sha256}, ${staged.runtime_size} bytes). Cached only; this action did not install or restart the runtime.`}
          </p>
        ) : null}
        <p className={styles.muted}>
          Activation is unavailable in this runtime coordinator: staging never
          installs, restarts or replaces the running runtime.
        </p>
        {actionError ? <FieldError>{actionError}</FieldError> : null}
      </Surface>
    </SettingsGroup>
  );
}
