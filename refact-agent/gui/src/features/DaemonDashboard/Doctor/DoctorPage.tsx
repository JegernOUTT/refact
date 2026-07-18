import { useCallback, useEffect, useMemo, useState } from "react";
import { RefreshCw, Stethoscope } from "lucide-react";

import {
  Button,
  EmptyState,
  FieldSelect,
  Icon,
  Surface,
} from "../../../components/ui";
import { selectConfig } from "../../Config/configSlice";
import { useAppDispatch, useAppSelector } from "../../../hooks";
import {
  resolveDaemonBaseUrl,
  useListProjectsQuery,
  useRestartProjectMutation,
  type DaemonWorker,
} from "../../../services/refact/daemon";
import { navigateDashboard } from "../dashboardSlice";
import {
  fetchServerFindings,
  runClientChecks,
  type DoctorFinding,
  type DoctorFix,
  type DoctorSeverity,
  type StaleModelFix,
} from "./clientChecks";
import { applyStaleModelFix } from "./fixActions";
import { FindingCard } from "./FindingCard";
import styles from "./Doctor.module.css";

const emptyWorkers: DaemonWorker[] = [];

const SEVERITY_SECTIONS: { severity: DoctorSeverity; title: string }[] = [
  { severity: "critical", title: "Critical" },
  { severity: "warning", title: "Warnings" },
];

const serverCheckFailedFinding: DoctorFinding = {
  id: "check_failed:daemon:doctor",
  severity: "info",
  message: "Check failed: daemon doctor report",
  detail:
    "The daemon doctor endpoint could not be reached. Run checks again once the daemon is reachable.",
  fix: null,
};

type FixStatus = "idle" | "applying" | "applied" | "error";

function FixOutcome({ status }: { status: FixStatus }) {
  if (status === "applied") {
    return <span className={styles.fixSuccess}>Applied — rechecking</span>;
  }
  if (status === "error") {
    return <span className={styles.fixError}>Fix failed. Try again.</span>;
  }
  return null;
}

function StaleModelFixControl({
  daemonBase,
  fix,
  onRecheck,
}: {
  daemonBase: string;
  fix: StaleModelFix;
  onRecheck: () => void;
}) {
  const [model, setModel] = useState(fix.availableModels[0] ?? "");
  const [status, setStatus] = useState<FixStatus>("idle");
  const options = fix.availableModels.map((value) => ({
    value,
    label: value,
  }));

  async function apply() {
    setStatus("applying");
    try {
      await applyStaleModelFix(daemonBase, fix, model);
      setStatus("applied");
      onRecheck();
    } catch {
      setStatus("error");
    }
  }

  return (
    <div className={styles.fixRow}>
      <FieldSelect
        aria-label={`Replacement model for ${fix.projectSlug}`}
        onChange={setModel}
        options={options}
        value={model}
      />
      <Button
        disabled={status === "applying" || model === ""}
        onClick={() => void apply()}
        size="sm"
      >
        {status === "applying" ? "Applying…" : "Apply"}
      </Button>
      <FixOutcome status={status} />
    </div>
  );
}

function RestartWorkerFixControl({
  projectId,
  onRecheck,
}: {
  projectId: string;
  onRecheck: () => void;
}) {
  const [restartProject, result] = useRestartProjectMutation();
  const [status, setStatus] = useState<FixStatus>("idle");

  async function restart() {
    setStatus("applying");
    try {
      await restartProject(projectId).unwrap();
      setStatus("applied");
      onRecheck();
    } catch {
      setStatus("error");
    }
  }

  return (
    <div className={styles.fixRow}>
      <Button
        disabled={result.isLoading}
        onClick={() => void restart()}
        size="sm"
      >
        {result.isLoading ? "Restarting…" : "Restart worker"}
      </Button>
      <FixOutcome status={status} />
    </div>
  );
}

function CopyCommandControl({
  command,
  hint,
  label,
}: {
  command: string;
  hint?: string;
  label?: string;
}) {
  const [status, setStatus] = useState<FixStatus>("idle");
  const copyLabel = label ?? "Copy command";

  async function copy() {
    try {
      await navigator.clipboard.writeText(command);
      setStatus("applied");
    } catch {
      setStatus("error");
    }
  }

  return (
    <div className={styles.fixColumn}>
      <div className={styles.fixRow}>
        <code className={styles.command}>{command}</code>
        <Button onClick={() => void copy()} size="sm" variant="outline">
          {status === "applied" ? "Copied" : copyLabel}
        </Button>
        {status === "error" ? (
          <span className={styles.fixError}>
            Copy failed. Copy it manually.
          </span>
        ) : null}
      </div>
      {hint ? <p className={styles.fixHint}>{hint}</p> : null}
    </div>
  );
}

function FindingFix({
  daemonBase,
  fix,
  onOpenSettings,
  onRecheck,
}: {
  daemonBase: string;
  fix: DoctorFix;
  onOpenSettings: () => void;
  onRecheck: () => void;
}) {
  switch (fix.kind) {
    case "stale_default_model":
      return (
        <StaleModelFixControl
          daemonBase={daemonBase}
          fix={fix}
          onRecheck={onRecheck}
        />
      );
    case "restart_worker":
      return (
        <RestartWorkerFixControl
          onRecheck={onRecheck}
          projectId={fix.projectId}
        />
      );
    case "run_update":
      return (
        <Button onClick={onOpenSettings} size="sm">
          Open updates
        </Button>
      );
    case "open_settings":
      return (
        <Button onClick={onOpenSettings} size="sm">
          Open settings
        </Button>
      );
    case "copy_command":
      return (
        <CopyCommandControl
          command={fix.command}
          hint={fix.hint}
          label={fix.label}
        />
      );
    case "open_project_providers":
      return (
        <a
          className={styles.providerLink}
          href={`/p/${encodeURIComponent(fix.projectId)}/?page=providers`}
        >
          Open provider settings
        </a>
      );
  }
}

export function DoctorPage() {
  const dispatch = useAppDispatch();
  const config = useAppSelector(selectConfig);
  const daemonBase = resolveDaemonBaseUrl(config);
  const { data: workersData, isLoading: workersLoading } =
    useListProjectsQuery(undefined);
  const workers = workersData ?? emptyWorkers;
  const [findings, setFindings] = useState<DoctorFinding[] | null>(null);
  const [running, setRunning] = useState(true);
  const [runToken, setRunToken] = useState(0);

  const recheck = useCallback(() => {
    setRunToken((token) => token + 1);
  }, []);

  const openSettings = useCallback(() => {
    dispatch(navigateDashboard({ page: "settings", params: {} }));
  }, [dispatch]);

  useEffect(() => {
    if (workersLoading) return;
    const controller = new AbortController();
    setRunning(true);
    void Promise.all([
      fetchServerFindings(daemonBase, controller.signal).catch(() => [
        serverCheckFailedFinding,
      ]),
      runClientChecks(daemonBase, workers, controller.signal),
    ]).then(([serverFindings, clientFindings]) => {
      if (controller.signal.aborted) return;
      setFindings([...serverFindings, ...clientFindings]);
      setRunning(false);
    });
    return () => controller.abort();
  }, [daemonBase, runToken, workers, workersLoading]);

  const sections = useMemo(
    () =>
      SEVERITY_SECTIONS.map((section) => ({
        ...section,
        findings: (findings ?? []).filter(
          (finding) => finding.severity === section.severity,
        ),
      })).filter((section) => section.findings.length > 0),
    [findings],
  );

  const infoFindings = useMemo(
    () => (findings ?? []).filter((finding) => finding.severity === "info"),
    [findings],
  );

  const allGreen =
    !running &&
    findings !== null &&
    findings.every((finding) => finding.severity === "info");

  return (
    <section className={styles.page} aria-labelledby="doctor-heading">
      <header className={styles.pageHeader}>
        <div>
          <h2 id="doctor-heading">Doctor</h2>
          <p>Daemon and project health checks with guided fixes.</p>
        </div>
        <Button disabled={running} onClick={recheck} variant="outline">
          <Icon icon={RefreshCw} size="sm" />
          {running ? "Running checks…" : "Run checks"}
        </Button>
      </header>

      {running && findings === null ? (
        <p className={styles.muted} aria-live="polite">
          Running checks…
        </p>
      ) : null}

      {allGreen ? (
        <EmptyState
          description="Daemon, workers, providers, and default models look healthy."
          icon={Stethoscope}
          title="All checks passed 🩺"
        />
      ) : null}

      {sections.map((section) => (
        <Surface
          as="section"
          aria-labelledby={`doctor-${section.severity}`}
          className={styles.group}
          key={section.severity}
          radius="card"
          variant="glass"
        >
          <h3 id={`doctor-${section.severity}`}>{section.title}</h3>
          <ul className={styles.findingList}>
            {section.findings.map((finding) => (
              <FindingCardWithFix
                daemonBase={daemonBase}
                finding={finding}
                key={finding.id}
                onOpenSettings={openSettings}
                onRecheck={recheck}
              />
            ))}
          </ul>
        </Surface>
      ))}

      {infoFindings.length > 0 ? (
        <Surface
          as="section"
          aria-label="Informational findings"
          className={styles.group}
          radius="card"
          variant="glass"
        >
          <details className={styles.infoDetails}>
            <summary className={styles.infoSummary}>
              Informational ({infoFindings.length})
            </summary>
            <ul className={styles.findingList}>
              {infoFindings.map((finding) => (
                <FindingCardWithFix
                  daemonBase={daemonBase}
                  finding={finding}
                  key={finding.id}
                  onOpenSettings={openSettings}
                  onRecheck={recheck}
                />
              ))}
            </ul>
          </details>
        </Surface>
      ) : null}
    </section>
  );
}

function FindingCardWithFix({
  daemonBase,
  finding,
  onOpenSettings,
  onRecheck,
}: {
  daemonBase: string;
  finding: DoctorFinding;
  onOpenSettings: () => void;
  onRecheck: () => void;
}) {
  return (
    <FindingCard
      action={
        finding.fix ? (
          <FindingFix
            daemonBase={daemonBase}
            fix={finding.fix}
            onOpenSettings={onOpenSettings}
            onRecheck={onRecheck}
          />
        ) : undefined
      }
      finding={finding}
    />
  );
}
