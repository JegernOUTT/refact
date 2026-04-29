import React from "react";
import classNames from "classnames";
import { BuddyCanvas } from "./BuddyCanvas";
import { SETUP_MODES } from "../Setup/setupModes";
import { getSignalDef } from "./constants";
import type {
  BuddyCareAction,
  BuddyControl,
  BuddyEvent,
  BuddyRuntimeEvent,
  BuddySemanticState,
  BuddySpeechItem,
  Palette,
  Stage,
} from "./types";
import styles from "./BuddyHome.module.css";

const CARE_ACTIONS: {
  action: BuddyCareAction;
  label: string;
  emoji: string;
  toy?: string;
}[] = [
  { action: "feed", label: "Feed", emoji: "🍜" },
  { action: "play", label: "Play", emoji: "🎾", toy: "bug" },
  { action: "pet", label: "Pet", emoji: "💕" },
  { action: "sleep", label: "Sleep", emoji: "😴" },
  { action: "clean", label: "Clean", emoji: "🧼" },
];

interface BuddyHeroProps {
  palette: Palette;
  stage: Stage;
  statusText: string;
  state: BuddySemanticState;
  onCanvasEvent: (event: BuddyEvent) => void;
  activeSpeech: BuddySpeechItem | null;
  nowPlaying: BuddyRuntimeEvent | null;
  setupNeeded: boolean;
  onRunMode: (mode: string) => void;
  onDismissSetup: () => void;
  onCare: (action: BuddyCareAction, toy?: string) => void;
  onSpeechControl: (control: BuddyControl) => void;
}

export const BuddyHero: React.FC<BuddyHeroProps> = ({
  palette,
  stage,
  statusText,
  state,
  onCanvasEvent,
  activeSpeech,
  nowPlaying,
  setupNeeded,
  onRunMode,
  onDismissSetup,
  onCare,
  onSpeechControl,
}) => (
  <div className={styles.hero} data-testid="buddy-hero">
    <div className={styles.scene}>
      <div className={styles.glowWrap}>
        <div
          className={styles.glow}
          style={{ backgroundColor: palette.body }}
        />
        <BuddyCanvas
          state={state}
          onEvent={onCanvasEvent}
          displaySize={200}
          speechOverride={
            activeSpeech
              ? activeSpeech.text
              : nowPlaying?.speech_text ?? nowPlaying?.title ?? null
          }
          speechControls={activeSpeech ? activeSpeech.controls : undefined}
          onSpeechControlClick={activeSpeech ? onSpeechControl : undefined}
        />
      </div>
    </div>

    <div
      className={styles.stageBadge}
      style={{
        backgroundColor: palette.body + "33",
        color: palette.body,
      }}
    >
      {stage.emoji} {stage.name}
    </div>

    {statusText && <div className={styles.statusText}>{statusText}</div>}

    {nowPlaying?.progress != null && (
      <div className={styles.statusBubble}>
        <span className={styles.statusIcon}>
          {getSignalDef(nowPlaying.signal_type).icon}
        </span>
        <div className={styles.statusContent}>
          <div className={styles.progressBar}>
            <div style={{ width: `${nowPlaying.progress}%` }} />
          </div>
        </div>
      </div>
    )}

    {setupNeeded && (
      <div className={styles.setupChips}>
        {SETUP_MODES.map((m) => (
          <button
            key={m.mode}
            type="button"
            className={classNames(styles.chip, {
              [styles.chipPrimary]: m.mode === "setup",
            })}
            onClick={() => onRunMode(m.mode)}
          >
            {m.label}
          </button>
        ))}
        <button
          type="button"
          className={classNames(styles.chip, styles.chipGhost)}
          onClick={onDismissSetup}
        >
          Dismiss
        </button>
      </div>
    )}

    <div className={styles.careBar}>
      {CARE_ACTIONS.map((item) => (
        <button
          key={item.action}
          type="button"
          className={styles.careButton}
          onClick={() => onCare(item.action, item.toy)}
        >
          <span>{item.emoji}</span>
          <span>{item.label}</span>
        </button>
      ))}
    </div>
  </div>
);
