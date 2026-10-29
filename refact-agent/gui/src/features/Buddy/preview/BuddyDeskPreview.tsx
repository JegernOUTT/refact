import { Badge, Button } from "@radix-ui/themes";
import { useEffect, useMemo, useRef, useState } from "react";
import { ChevronLeft, Gamepad2, Sparkles } from "lucide-react";
import { BuddyCanvas } from "../BuddyCanvas";
import type { BuddySemanticState } from "../types";
import {
  buddyDeskFixtures,
  laneLabels,
  type BuddyDeskItem,
} from "./buddyDeskFixtures";
import styles from "./BuddyDeskPreview.module.css";

type ActionStatus = "idle" | "pending" | "success" | "partial" | "failure";

const lanes = ["needs", "working", "delivered", "watching"] as const;

const quietBuddy: BuddySemanticState = {
  name: "Pixel",
  paletteIndex: 0,
  born: 0,
  mood: {
    happiness: 74,
    energy: 70,
    curiosity: 68,
    anxiety: 8,
    boredom: 15,
    affection: 60,
  },
  personality: {
    playfulness: 68,
    confidence: 62,
    clinginess: 35,
    resilience: 72,
    chaos: 28,
    sociability: 55,
    curiosity: 70,
  },
  progress: { xp: 120, stage: 2 },
  activity: {
    mood: "focused",
    animationType: "idle",
    lastSignalTime: 0,
    lastSignalType: null,
  },
  skills: ["test"],
  log: [],
};

function stateLabel(item: BuddyDeskItem) {
  return item.state === "watching" ? "No change" : item.state;
}

function ReportArtifact() {
  return (
    <section className={styles.artifact} aria-label="Readable report artifact">
      <h3>Keyboard navigation report</h3>
      <ul>
        <li>Tab reaches each work item and its stable action.</li>
        <li>Enter opens readable detail without opening chat.</li>
        <li>Focus remains visible throughout this simulated preview.</li>
      </ul>
      <p>Result: verified in this simulated preview only.</p>
    </section>
  );
}

export function BuddyDeskPreview() {
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [actionStatus, setActionStatus] = useState<
    Record<string, ActionStatus>
  >({});
  const [playState, setPlayState] = useState(
    "Pixel is taking a quiet play break.",
  );
  const [buddyState, setBuddyState] = useState(quietBuddy);
  const backFocusRef = useRef<HTMLButtonElement | null>(null);
  const mountedRef = useRef(true);
  const actionTimersRef = useRef(new Map<string, number>());
  const playTimerRef = useRef<number | null>(null);
  const selected =
    buddyDeskFixtures.find((item) => item.id === selectedId) ?? null;
  const status: ActionStatus =
    selected && selected.id in actionStatus
      ? actionStatus[selected.id]
      : "idle";
  const grouped = useMemo(
    () =>
      Object.fromEntries(
        lanes.map((lane) => [
          lane,
          buddyDeskFixtures.filter((item) => item.lane === lane),
        ]),
      ),
    [],
  );

  useEffect(() => {
    const actionTimers = actionTimersRef.current;
    return () => {
      mountedRef.current = false;
      actionTimers.forEach((timer) => window.clearTimeout(timer));
      if (playTimerRef.current !== null)
        window.clearTimeout(playTimerRef.current);
    };
  }, []);

  const selectItem = (id: string, trigger: HTMLButtonElement) => {
    backFocusRef.current = trigger;
    setSelectedId(id);
  };

  const closeDetail = () => {
    setSelectedId(null);
    window.requestAnimationFrame(() => backFocusRef.current?.focus());
  };

  const runAction = (item: BuddyDeskItem) => {
    const action = item.action;
    if (!action || actionTimersRef.current.has(item.id)) return;
    setActionStatus((current) => ({ ...current, [item.id]: "pending" }));
    const timer = window.setTimeout(() => {
      actionTimersRef.current.delete(item.id);
      if (mountedRef.current) {
        setActionStatus((current) => ({
          ...current,
          [item.id]: action.outcome,
        }));
      }
    }, 250);
    actionTimersRef.current.set(item.id, timer);
  };

  const tossBall = () => {
    if (playTimerRef.current !== null)
      window.clearTimeout(playTimerRef.current);
    setPlayState(
      "Pixel tossed the virtual ball — a local, simulated play break.",
    );
    setBuddyState((current) => ({
      ...current,
      activity: {
        mood: "happy",
        animationType: "celebrate",
        lastSignalTime: Date.now(),
        lastSignalType: "care_play",
      },
    }));
    playTimerRef.current = window.setTimeout(() => {
      if (!mountedRef.current) return;
      setPlayState("Pixel returned to a quiet play break.");
      setBuddyState((current) => ({
        ...current,
        activity: quietBuddy.activity,
      }));
      playTimerRef.current = null;
    }, 1_200);
  };

  return (
    <main
      className={styles.preview}
      data-detail-open={String(Boolean(selected))}
      data-testid="buddy-desk-preview"
    >
      <header className={styles.header}>
        <div className={styles.identity}>
          <div
            className={styles.avatar}
            aria-label="Pixel Buddy avatar, present and focused"
            role="img"
          >
            <BuddyCanvas state={buddyState} displaySize={76} />
          </div>
          <div>
            <p className={styles.eyebrow}>Reports</p>
            <h1>Pixel is focused</h1>
            <p className={styles.presence}>
              Present · sorting fixture evidence, not changing your project
            </p>
          </div>
        </div>
        <Badge className={styles.previewLabel} color="gray" variant="soft">
          <Sparkles size={14} aria-hidden="true" /> Preview — simulated actions
        </Badge>
      </header>

      <section className={styles.play} aria-label="Buddy play">
        <Gamepad2 size={18} aria-hidden="true" />
        <span role="status">{playState}</span>
        <Button type="button" variant="soft" onClick={tossBall}>
          Toss ball (simulated)
        </Button>
      </section>

      <div className={styles.workspace}>
        <section className={styles.list} aria-label="Buddy work list">
          {lanes.map((lane) => (
            <section
              className={styles.lane}
              key={lane}
              aria-labelledby={`lane-${lane}`}
            >
              <h2 id={`lane-${lane}`}>{laneLabels[lane]}</h2>
              <ul>
                {grouped[lane].map((item) => (
                  <li key={item.id}>
                    <button
                      type="button"
                      className={
                        selected?.id === item.id
                          ? styles.itemSelected
                          : styles.item
                      }
                      onClick={(event) =>
                        selectItem(item.id, event.currentTarget)
                      }
                      aria-current={
                        selected?.id === item.id ? "true" : undefined
                      }
                    >
                      <Badge
                        className={styles.state}
                        color={
                          item.state === "blocked"
                            ? "red"
                            : item.state === "verified"
                              ? "green"
                              : item.state === "partial"
                                ? "amber"
                                : "gray"
                        }
                        variant="soft"
                      >
                        {stateLabel(item)}
                      </Badge>
                      <strong>{item.title}</strong>
                      <span>{item.summary}</span>
                    </button>
                  </li>
                ))}
              </ul>
            </section>
          ))}
        </section>

        <article className={styles.detail} aria-live="polite">
          {selected ? (
            <>
              <Button
                type="button"
                className={styles.back}
                variant="ghost"
                onClick={closeDetail}
              >
                <ChevronLeft size={16} aria-hidden="true" /> Back to list
              </Button>
              <Badge
                className={styles.state}
                color={
                  selected.state === "blocked"
                    ? "red"
                    : selected.state === "verified"
                      ? "green"
                      : selected.state === "partial"
                        ? "amber"
                        : "gray"
                }
                variant="soft"
              >
                {stateLabel(selected)}
              </Badge>
              <h2>{selected.title}</h2>
              <p>{selected.summary}</p>
              <section className={styles.evidence} aria-label="Evidence">
                <h3>Evidence</h3>
                <p>{selected.evidence}</p>
              </section>
              {selected.artifact && <ReportArtifact />}
              {selected.action && (
                <div className={styles.actionArea}>
                  <Button
                    type="button"
                    onClick={() => runAction(selected)}
                    disabled={status === "pending"}
                  >
                    {selected.action.label}
                  </Button>
                  <p role="status">
                    {status === "pending" && "Simulated action pending…"}
                    {status !== "idle" &&
                      status !== "pending" &&
                      selected.action.result}
                  </p>
                </div>
              )}
            </>
          ) : (
            <p className={styles.emptyDetail}>
              Choose an item to inspect its fixture evidence.
            </p>
          )}
        </article>
      </div>
    </main>
  );
}
