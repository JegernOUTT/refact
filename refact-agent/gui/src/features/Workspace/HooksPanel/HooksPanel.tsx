import { useEffect } from "react";
import { RefreshCw, Webhook } from "lucide-react";
import {
  Badge,
  Button,
  EmptyState,
  ErrorState,
  IconButton,
  LoadingState,
} from "../../../components/ui";
import {
  hookSurfaceApi,
  useGetHookSurfaceQuery,
} from "../../../services/refact/hookSurface";
import {
  decidingRun,
  isDenial,
  type HookActivity,
  type HookRegistration,
  type HookSurface,
} from "../../../services/refact/hookSurfaceTypes";
import { humanizeIdentifier } from "../../../utils/displayNames";
import { panelErrorDescription } from "../panelError";
import { useAppDispatch } from "../../../hooks";
import { usePanelScope } from "../usePanelScope";
import styles from "./HooksPanel.module.css";

function verdictLabel(activity: HookActivity): string {
  if (activity.verdict === "recursion_blocked") return "Recursion cap reached";
  if (activity.verdict === "rewrite_refused") return "Rewrite refused";
  if (activity.verdict === "rewrite_accepted") return "Rewrite revalidated";
  return humanizeIdentifier(activity.verdict);
}

function RegistrationItem({ entry }: { entry: HookRegistration }) {
  return (
    <li className={styles.item}>
      <div className={styles.heading}>
        <strong className={styles.command}>{entry.command}</strong>
        <Badge tone={entry.runs ? "muted" : "warning"}>
          {entry.runs ? "Runs" : "Never executed"}
        </Badge>
      </div>
      <span className={styles.muted}>
        {entry.event} · {humanizeIdentifier(entry.kind)} ·{" "}
        {humanizeIdentifier(entry.trust)}
      </span>
      <span className={styles.muted}>{entry.source}</span>
      {entry.matcher !== undefined && (
        <span className={styles.muted}>Matches {entry.matcher}</span>
      )}
      {entry.blocked_reason !== undefined && (
        <p className={styles.warning} role="status">
          {entry.blocked_reason}
        </p>
      )}
    </li>
  );
}

function ActivityItem({ activity }: { activity: HookActivity }) {
  const decided = decidingRun(activity);
  return (
    <li className={styles.item}>
      <div className={styles.heading}>
        <strong className="rf-wrap-anywhere">{activity.event}</strong>
        <Badge tone={isDenial(activity) ? "warning" : "muted"}>
          {verdictLabel(activity)}
        </Badge>
      </div>
      {activity.tool !== undefined && (
        <span className={styles.muted}>Tool: {activity.tool}</span>
      )}
      {decided ? (
        <p className={styles.warning} aria-label="Deciding hook">
          {decided.command} decided this: {humanizeIdentifier(decided.outcome)}.
          Deny outranks every other answer.
        </p>
      ) : (
        activity.verdict === "allow" && (
          <p className={styles.muted}>No hook objected.</p>
        )
      )}
      {activity.reason !== undefined && (
        <p className={styles.warning}>{activity.reason}</p>
      )}
      {activity.runs.length > 0 && (
        <ul
          className={styles.runs}
          aria-label={`Hooks that ran for ${activity.event}`}
        >
          {activity.runs.map((run) => (
            <li
              className={styles.command}
              key={`${run.command}:${run.outcome}`}
            >
              {run.command} returned {humanizeIdentifier(run.outcome)}
              {run.decided ? " (decided)" : ""}
              {run.detail !== undefined ? `: ${run.detail}` : ""}
            </li>
          ))}
        </ul>
      )}
    </li>
  );
}

function SurfaceSummary({ surface }: { surface: HookSurface }) {
  return (
    <>
      <p role="status" className={styles.muted}>
        {surface.registered} registered · {surface.runnable} run ·{" "}
        {surface.blocked} never executed
      </p>
      {!surface.enabled && (
        <p role="status" className={styles.warning}>
          Hooks are disabled, so nothing registered here is executed.
        </p>
      )}
      {surface.denials > 0 && (
        <p role="status" aria-label="Recent denials" className={styles.warning}>
          {surface.denials} recent event{surface.denials === 1 ? "" : "s"} were
          denied or refused.
        </p>
      )}
    </>
  );
}

export function HooksPanel() {
  const endpoint = usePanelScope();
  return <HooksContents key={endpoint} endpoint={endpoint} />;
}

function HooksContents({ endpoint }: { endpoint: string }) {
  const dispatch = useAppDispatch();
  const {
    currentData: surface,
    isFetching,
    error,
    refetch,
  } = useGetHookSurfaceQuery({ endpoint });
  useEffect(
    () => () => {
      dispatch(
        hookSurfaceApi.util.getRunningQueryThunk("getHookSurface", {
          endpoint,
        }),
      )?.abort();
    },
    [dispatch, endpoint],
  );
  useEffect(() => {
    if (!error) return;
    dispatch(
      hookSurfaceApi.util.updateQueryData(
        "getHookSurface",
        { endpoint },
        (draft) => {
          draft.registrations = [];
          draft.recent = [];
          draft.registered = 0;
          draft.runnable = 0;
          draft.blocked = 0;
          draft.denials = 0;
        },
      ),
    );
  }, [error, dispatch, endpoint]);
  return (
    <section className={styles.panel} aria-label="Hooks">
      <header className={styles.heading}>
        <h2>Hooks</h2>
        <IconButton
          aria-label="Refresh hooks"
          icon={RefreshCw}
          variant="ghost"
          size="sm"
          onClick={() => void refetch()}
          disabled={isFetching}
        />
      </header>
      <p className={styles.muted}>
        Which hooks are registered, which are trusted, and what the ones that
        ran returned. Read-only.
      </p>
      {error ? (
        <ErrorState
          title="Hooks unavailable"
          description={panelErrorDescription(error)}
          retry={<Button onClick={() => void refetch()}>Refresh</Button>}
        />
      ) : !surface || isFetching ? (
        <LoadingState label="Loading hooks" />
      ) : (
        <>
          <SurfaceSummary surface={surface} />
          <div className={styles.body}>
            {surface.registrations.length === 0 ? (
              <EmptyState
                icon={Webhook}
                title="No hooks registered"
                description="No hook is registered for this project or globally."
                variant="compact"
              />
            ) : (
              <ul className={styles.list} aria-label="Registered hooks">
                {surface.registrations.map((entry) => (
                  <RegistrationItem
                    key={`${entry.event}:${entry.command}:${entry.source}`}
                    entry={entry}
                  />
                ))}
              </ul>
            )}
            <section aria-label="Recent hook activity">
              <div className={styles.heading}>
                <strong>Recent activity</strong>
                <Badge tone={surface.denials > 0 ? "warning" : "muted"}>
                  {surface.recent.length} recorded
                </Badge>
              </div>
              {surface.recent.length === 0 ? (
                <p className={styles.muted}>
                  No hook has fired since this engine started.
                </p>
              ) : (
                <ul className={styles.list}>
                  {surface.recent.map((activity, index) => (
                    <ActivityItem
                      key={`${activity.at_ms}:${activity.event}:${index}`}
                      activity={activity}
                    />
                  ))}
                </ul>
              )}
            </section>
          </div>
        </>
      )}
    </section>
  );
}
