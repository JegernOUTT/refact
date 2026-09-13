import { useCallback, useEffect, useRef, useState } from "react";
import type { KeyboardEvent as ReactKeyboardEvent } from "react";
import { useReducedMotion } from "../../hooks";
import {
  advanceBuddyPlaySession,
  createFetchSession,
  createFireflySession,
  describePlaySessionForScreenReader,
  playKeyboardTargetForKey,
  playKeyboardTargets,
  playSessionCompletion,
  pounceFirefly,
  resumePlaySession,
  suspendPlaySession,
  throwFetchBall,
  type BuddyPlayCompletion,
  type BuddyPlaySession,
  type BuddyPlaySessionKind,
  type BuddyPlaySuspension,
} from "./buddyPlaySessions";
import {
  buddyPresenceMotion,
  resolveBuddyContinuity,
  resolveBuddyMotionTransition,
  resolveBuddyPresenceLoop,
  type BuddyWorkState,
  useBuddyPresencePreferences,
} from "./buddyPresenceMotion";
import styles from "./BuddyOperatorPlayPreview.module.css";

const PHASE_TICK_MS = 100;
const PREVIEW_BUDDY_X = 50;

export interface BuddyOperatorPlayPreviewProps {
  /**
   * Default-disabled (plan §16, B35). The component renders nothing unless a
   * caller opts in explicitly; it is never enabled by importing it.
   */
  enabled?: boolean;
  /** Work state driving the motion label; play never changes work itself. */
  workState?: BuddyWorkState;
  /** Overrides the media query; omitted means follow the user's OS setting. */
  reducedMotion?: boolean;
  /** Host low-power hint. No live host integration exists yet. */
  lowPower?: boolean;
  /** Injected for deterministic tests; defaults to the real clock. */
  nowMs?: () => number;
  lastSeenAtMs?: number;
}

export function BuddyOperatorPlayPreview({
  enabled = false,
  workState: workStateOverride,
  reducedMotion,
  lowPower = false,
  nowMs,
  lastSeenAtMs,
}: BuddyOperatorPlayPreviewProps) {
  const presence = useBuddyPresencePreferences();
  const workState = workStateOverride ?? presence.workState;
  const systemReducedMotion = useReducedMotion();
  const effectiveReducedMotion = reducedMotion ?? systemReducedMotion;
  const clockRef = useRef(nowMs);
  clockRef.current = nowMs;
  const now = useCallback(() => clockRef.current?.() ?? Date.now(), []);

  const [session, setSession] = useState<BuddyPlaySession | null>(null);
  const [suspension, setSuspension] = useState<BuddyPlaySuspension | null>(
    null,
  );
  const [completion, setCompletion] = useState<BuddyPlayCompletion | null>(
    null,
  );
  const [documentHidden, setDocumentHidden] = useState(false);
  const surfaceRef = useRef<HTMLElement | null>(null);
  const startFocusRef = useRef<HTMLButtonElement | null>(null);
  const restoreFocusRef = useRef(false);
  const previousWorkStateRef = useRef<BuddyWorkState>(workState);
  const [remember, setRemember] = useState(false);
  const rememberRef = useRef(false);
  const [savedLastSeen, setSavedLastSeen] = useState<number>();
  const [exportedContinuity, setExportedContinuity] = useState("");
  const continuityKey = "buddy.presence.preview.lastSeen.v1";
  useEffect(() => {
    if (!enabled || !remember) return;
    try {
      const raw = localStorage.getItem(continuityKey);
      const value = raw !== null && raw.length <= 32 ? Number(raw) : NaN;
      if (Number.isSafeInteger(value) && value >= 0 && value <= now())
        setSavedLastSeen(value);
    } catch {
      /* Storage may be unavailable; continuity remains memory-only. */
    }
    const save = () => {
      if (!rememberRef.current) return;
      try {
        localStorage.setItem(
          continuityKey,
          String(Math.max(0, Math.floor(now()))),
        );
      } catch {
        /* Optional only. */
      }
    };
    document.addEventListener("visibilitychange", save);
    return () => {
      document.removeEventListener("visibilitychange", save);
      save();
    };
  }, [enabled, remember, now]);

  useEffect(() => {
    if (!enabled) return;
    const sync = () => setDocumentHidden(document.hidden);
    sync();
    document.addEventListener("visibilitychange", sync);
    return () => document.removeEventListener("visibilitychange", sync);
  }, [enabled]);

  const loop = resolveBuddyPresenceLoop({
    mounted: enabled,
    documentHidden,
    enabled,
    reducedMotion: effectiveReducedMotion,
    lowPower,
    interactive: session !== null,
  });

  const sessionRef = useRef<BuddyPlaySession | null>(session);
  sessionRef.current = session;
  const suspensionRef = useRef<BuddyPlaySuspension | null>(suspension);
  suspensionRef.current = suspension;

  const suspendFor = useCallback(
    (reason: BuddyPlaySuspension["reason"]) => {
      const current = sessionRef.current;
      if (!current) return;
      // Pause disables its button and removes armed targets. Keep keyboard
      // ownership only when the user is already interacting with this surface.
      if (
        reason === "user_paused" &&
        surfaceRef.current?.contains(document.activeElement)
      )
        surfaceRef.current.focus();
      setSuspension(suspendPlaySession(current, now(), reason));
      setSession(null);
    },
    [now],
  );

  useEffect(() => {
    if (!enabled) return;
    if (!documentHidden) return;
    suspendFor("hidden");
  }, [documentHidden, enabled, suspendFor]);

  useEffect(() => {
    if (enabled && buddyPresenceMotion(workState).ownsAttention)
      suspendFor("work_took_over");
  }, [enabled, workState, suspendFor]);

  useEffect(() => {
    if (!enabled) return;
    return () => {
      setSession(null);
      setSuspension(null);
    };
  }, [enabled]);

  useEffect(() => {
    if (!enabled || session === null) return;
    if (!loop.running) return;
    const timer = window.setInterval(() => {
      setSession((current) => {
        if (!current) return current;
        const advanced = advanceBuddyPlaySession(current, now());
        if (!advanced) {
          setCompletion(playSessionCompletion(current, "finished"));
          return null;
        }
        if (advanced.phase === "done" && current.phase !== "done") {
          setCompletion(playSessionCompletion(advanced, "finished"));
        }
        return advanced;
      });
    }, PHASE_TICK_MS);
    return () => window.clearInterval(timer);
  }, [enabled, loop.running, now, session]);

  const startPlay = useCallback(
    (kind: BuddyPlaySessionKind) => {
      if (document.hidden || buddyPresenceMotion(workState).ownsAttention)
        return;
      // Start becomes disabled on the next render. A stable focus owner keeps
      // subsequent keyboard events inside this preview, rather than on body.
      surfaceRef.current?.focus();
      setCompletion(null);
      setSuspension(null);
      setSession(
        kind === "fetch"
          ? createFetchSession(now())
          : createFireflySession(now()),
      );
    },
    [now, workState],
  );

  const aimAt = useCallback(
    (x: number, y: number) => {
      if (sessionRef.current?.phase !== "armed") return;
      // The target buttons unmount after aiming; hand off before removal.
      surfaceRef.current?.focus();
      setSession((current) => {
        if (!current || current.phase !== "armed") return current;
        return current.kind === "fetch"
          ? throwFetchBall(current, x, y, PREVIEW_BUDDY_X, now())
          : pounceFirefly(current, x, y, now());
      });
    },
    [now],
  );

  const interruptPlay = useCallback(() => {
    suspendFor("user_paused");
  }, [suspendFor]);

  const resumePlay = useCallback(() => {
    const current = suspensionRef.current;
    if (
      !current ||
      document.hidden ||
      buddyPresenceMotion(workState).ownsAttention
    )
      return;
    surfaceRef.current?.focus();
    setSession(resumePlaySession(current, now()));
    setSuspension(null);
  }, [now, workState]);

  const stopPlay = useCallback(() => {
    const active = session ?? suspension?.session ?? null;
    if (active) setCompletion(playSessionCompletion(active, "interrupted"));
    setSession(null);
    setSuspension(null);
    restoreFocusRef.current = true;
  }, [session, suspension]);

  useEffect(() => {
    if (!restoreFocusRef.current) return;
    if (session !== null || suspension !== null) return;
    restoreFocusRef.current = false;
    startFocusRef.current?.focus();
  }, [session, suspension]);

  const activeSession = session ?? suspension?.session ?? null;

  const onKeyDown = useCallback(
    (event: ReactKeyboardEvent<HTMLElement>) => {
      if (
        event.repeat ||
        event.altKey ||
        event.ctrlKey ||
        event.metaKey ||
        (event.target instanceof HTMLElement &&
          event.target.matches(
            "input, textarea, select, [contenteditable=true]",
          ))
      )
        return;
      if (event.key === "Escape") {
        if (session) {
          event.preventDefault();
          interruptPlay();
        }
        return;
      }
      if (
        event.key === "Enter" &&
        suspension &&
        event.target === event.currentTarget
      ) {
        event.preventDefault();
        resumePlay();
        return;
      }
      if (!session || session.phase !== "armed") return;
      const target = playKeyboardTargetForKey(session.kind, event.key);
      if (!target) return;
      event.preventDefault();
      aimAt(target.x, target.y);
    },
    [aimAt, interruptPlay, resumePlay, session, suspension],
  );

  if (!enabled) return null;

  const motion = buddyPresenceMotion(workState);
  // A committed (non-armed) play phase locks motion, so only a higher-priority
  // work state may preempt it; equal or lower states are rejected, not queued.
  const transition = resolveBuddyMotionTransition(
    previousWorkStateRef.current,
    workState,
    {
      reducedMotion: effectiveReducedMotion,
      currentLocked: session !== null && session.phase !== "armed",
    },
  );
  if (transition.kind !== "rejected") {
    previousWorkStateRef.current = workState;
  }
  const continuity = resolveBuddyContinuity({
    lastSeenAtMs: lastSeenAtMs ?? savedLastSeen ?? now(),
    nowMs: now(),
  });
  const status = describePlaySessionForScreenReader(
    activeSession,
    suspension !== null,
  );

  return (
    <section
      ref={surfaceRef}
      tabIndex={-1}
      className={styles.preview}
      aria-label="Buddy short play preview"
      data-testid="buddy-operator-play-preview"
      data-loop-running={String(loop.running)}
      data-loop-stop={loop.stopReason ?? "none"}
      data-work-state={motion.state}
      data-pose={motion.pose}
      data-motion-transition={transition.kind}
      data-motion-transition-ms={transition.durationMs}
      data-reduced-motion={String(effectiveReducedMotion)}
      data-low-power={String(lowPower)}
      data-play={
        activeSession ? `${activeSession.kind}:${activeSession.phase}` : "none"
      }
      data-suspended={String(suspension !== null)}
      onKeyDown={onKeyDown}
    >
      <header className={styles.header}>
        <p className={styles.motion}>{motion.label}</p>
        <p className={styles.notice}>
          Preview only — play never changes your project or permissions.
        </p>
      </header>

      <p className={styles.continuity}>{continuity.label}</p>
      <label>
        <input
          type="checkbox"
          checked={remember}
          onChange={(event) => {
            rememberRef.current = event.target.checked;
            setRemember(event.target.checked);
          }}
        />
        Remember last seen on this device (preview only)
      </label>
      <button
        type="button"
        className={styles.control}
        onClick={() => {
          rememberRef.current = false;
          setRemember(false);
          setSavedLastSeen(undefined);
          setExportedContinuity("");
          try {
            localStorage.removeItem(continuityKey);
          } catch {
            /* Optional only. */
          }
        }}
      >
        Reset continuity
      </button>
      <button
        type="button"
        className={styles.control}
        onClick={() =>
          setExportedContinuity(
            JSON.stringify({ version: 1, lastSeenAtMs: savedLastSeen ?? null }),
          )
        }
      >
        Export continuity
      </button>
      {exportedContinuity && (
        <pre aria-label="Continuity export">{exportedContinuity}</pre>
      )}

      <div className={styles.controls}>
        <button
          type="button"
          ref={startFocusRef}
          className={styles.control}
          onClick={() => startPlay("fetch")}
          disabled={session !== null || suspension !== null}
        >
          Start fetch
        </button>
        <button
          type="button"
          className={styles.control}
          onClick={() => startPlay("firefly")}
          disabled={session !== null || suspension !== null}
        >
          Start firefly
        </button>
        <button
          type="button"
          className={styles.control}
          onClick={interruptPlay}
          disabled={session === null}
        >
          Pause play
        </button>
        <button
          type="button"
          className={styles.control}
          onClick={resumePlay}
          disabled={suspension === null}
        >
          Resume play
        </button>
        <button
          type="button"
          className={styles.control}
          onClick={stopPlay}
          disabled={activeSession === null}
        >
          Stop play
        </button>
      </div>

      {session?.phase === "armed" && (
        <div
          className={styles.targets}
          aria-label={
            session.kind === "fetch" ? "Throw targets" : "Pounce targets"
          }
        >
          {playKeyboardTargets(session.kind).map((target) => (
            <button
              key={target.id}
              type="button"
              className={styles.target}
              onClick={() => aimAt(target.x, target.y)}
            >
              <span aria-hidden="true" className={styles.shortcut}>
                {target.key}
              </span>
              {target.label}
            </button>
          ))}
        </div>
      )}

      <p className={styles.status} role="status" aria-live="polite">
        {status}
      </p>
      {completion && (
        <p className={styles.completion} data-outcome={completion.outcome}>
          {completion.label}
        </p>
      )}
      {!loop.running && (
        <p className={styles.paused} data-testid="buddy-play-loop-paused">
          Animation paused ({loop.stopReason ?? "unknown"}). Play state is kept.
        </p>
      )}
    </section>
  );
}
