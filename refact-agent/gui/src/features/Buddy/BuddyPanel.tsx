import React, { useCallback, useMemo } from "react";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { push } from "../Pages/pagesSlice";
import { BuddyCanvas } from "./BuddyCanvas";
import { BuddySpeechCloud } from "./BuddySpeechCloud";
import { useBuddyState } from "./hooks/useBuddyState";
import {
  selectBuddySnapshot,
  selectIsBuddyEnabled,
  selectNowPlaying,
} from "./buddySlice";
import { PALETTES, STAGES, SIGNALS } from "./constants";
import { computeXpFill } from "./buddyUtils";
import styles from "./BuddyPanel.module.css";

export const BuddyPanel: React.FC = () => {
  const dispatch = useAppDispatch();
  const snapshot = useAppSelector(selectBuddySnapshot);
  const enabled = useAppSelector(selectIsBuddyEnabled);
  const nowPlaying = useAppSelector(selectNowPlaying);

  const buddy = useBuddyState();
  const { state } = buddy;

  const paletteIndex =
    snapshot?.state.identity.palette_index ?? state.paletteIndex;
  const palette = PALETTES[paletteIndex] ?? PALETTES[0];

  const progression = snapshot?.state.progression;
  const identity = snapshot?.state.identity;

  const stageIdx = progression?.stage ?? state.progress.stage;
  const stage = STAGES[stageIdx] ?? STAGES[0];

  const xp = progression?.xp ?? state.progress.xp;

  const xpFill = useMemo(
    () => computeXpFill(progression?.xp ?? 0, progression?.xp_next ?? 100),
    [progression],
  );

  const name = identity?.name ?? state.name;

  const handleOpen = useCallback(() => {
    dispatch(push({ name: "buddy" }));
  }, [dispatch]);

  if (snapshot === null) return null;
  if (!enabled) return null;

  return (
    <div
      className={styles.block}
      onClick={handleOpen}
      style={{ cursor: "pointer" }}
    >
      <div className={styles.body}>
        <div className={styles.scene}>
          <div className={styles.glowWrap}>
            <div
              className={styles.glow}
              style={{ backgroundColor: palette.body }}
            />
            <BuddyCanvas
              state={state}
              onEvent={buddy.handleCanvasEvent}
              displaySize={200}
            />
          </div>
          <BuddySpeechCloud variant="overlay" />
        </div>

        <div className={styles.info}>
          <div className={styles.nameRow}>
            <span className={styles.name}>{name}</span>
            <span
              className={styles.stageBadge}
              style={{
                backgroundColor: palette.body + "33",
                color: palette.body,
              }}
            >
              {stage.emoji} {stage.name}
            </span>
            <div className={styles.xpBarInline}>
              <div
                className={styles.xpFillInline}
                style={{ width: `${xpFill}%` }}
              />
            </div>
            <span className={styles.xpText}>{xp}</span>
          </div>

          {nowPlaying && (
            <div className={styles.statusBubble}>
              <span className={styles.statusIcon}>
                {SIGNALS[nowPlaying.signal_type]?.icon ?? "⚡"}
              </span>
              <span className={styles.statusTitle}>{nowPlaying.title}</span>
              {nowPlaying.progress != null && (
                <div className={styles.progressBar}>
                  <div style={{ width: `${nowPlaying.progress}%` }} />
                </div>
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  );
};
