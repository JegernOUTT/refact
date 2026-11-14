import * as Collapsible from "@radix-ui/react-collapsible";
import { Button } from "../../components/ui";
import { useAppSelector } from "../../hooks/useAppSelector";
import { selectHost, selectRuntimeVersions } from "../Config/configSlice";
import {
  boundedVersionString,
  sanitizeObservation,
  type RuntimeVersionObservation,
} from "./types";
import styles from "./RuntimeVersions.module.css";

const sources = {
  plugin: "IDE plugin",
  local: "Local executable",
  daemon: "Daemon status",
  worker: "Project engine status",
};

function Observation({ value }: { value?: RuntimeVersionObservation | null }) {
  const observation = sanitizeObservation(value);
  if (!observation) return <>Not detected</>;
  return (
    <>
      <span>{observation.version ?? "Unknown"}</span>
      {observation.commit && (
        <span className={styles.metadata}>Commit {observation.commit}</span>
      )}
      {observation.source && (
        <span className={styles.metadata}>{sources[observation.source]}</span>
      )}
      <span className={styles.metadata}>
        {observation.state === "last_observed" ? "Last observed" : "Observed"}
        {observation.observedAtMs !== undefined && (
          <>
            {" "}
            ·{" "}
            <time dateTime={new Date(observation.observedAtMs).toISOString()}>
              {new Date(observation.observedAtMs).toLocaleString()}
            </time>
          </>
        )}
      </span>
    </>
  );
}

export function RuntimeVersionsSummary({
  daemonOverride,
}: {
  daemonOverride?: RuntimeVersionObservation | null;
}) {
  const versions = useAppSelector(selectRuntimeVersions);
  const host = useAppSelector(selectHost);
  const loaded =
    typeof __REFACT_CHAT_VERSION__ === "undefined"
      ? undefined
      : __REFACT_CHAT_VERSION__;
  const uiVersion = boundedVersionString(loaded?.semver);
  const uiCommit = boundedVersionString(loaded?.commit);
  const slots = [
    ["IDE plugin", versions?.plugin],
    ["Selected executable", versions?.selected],
    ["Installed runtime", versions?.installed],
    ["Staged runtime", versions?.staged],
    [
      "Running daemon",
      daemonOverride === undefined ? versions?.daemon : daemonOverride,
    ],
    ["Project engine", versions?.worker],
  ] as const;
  return (
    <Collapsible.Root className={styles.root}>
      <Collapsible.Trigger asChild>
        <Button variant="plain" size="sm">
          Component versions
        </Button>
      </Collapsible.Trigger>
      <Collapsible.Content>
        <dl className={styles.list}>
          <div className={styles.row}>
            <dt>{slots[0][0]}</dt>
            <dd>
              <Observation value={slots[0][1]} />
            </dd>
          </div>
          <div className={styles.row}>
            <dt>{host === "web" ? "Web UI" : "Plugin UI"}</dt>
            <dd>
              {uiVersion ?? "Unknown"}
              {uiCommit && (
                <span className={styles.metadata}>Commit {uiCommit}</span>
              )}
              <span className={styles.metadata}>Loaded UI bundle</span>
            </dd>
          </div>
          <div className={styles.row}>
            <dt>Requested runtime</dt>
            <dd>
              {boundedVersionString(versions?.requestedVersion) ?? "Unknown"}
              <span className={styles.metadata}>
                Policy, not an observed version
              </span>
            </dd>
          </div>
          {slots.slice(1).map(([label, value]) => (
            <div className={styles.row} key={label}>
              <dt>{label}</dt>
              <dd>
                <Observation value={value} />
              </dd>
            </div>
          ))}
        </dl>
      </Collapsible.Content>
    </Collapsible.Root>
  );
}
