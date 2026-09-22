import { useEffect, useRef, useState } from "react";
import { Button, Text } from "../../components/ui";
import { BuddyCanvas } from "./BuddyCanvas";
import { BuddyErrorBoundary } from "./BuddyErrorBoundary";
import type { BuddySceneSpeech } from "./buddySceneSpeech";
import type { BuddyControl, BuddySemanticState } from "./types";
import styles from "./BuddyPresence.module.css";
import { useBuddyTextPresence } from "./useBuddyTextPresence";

export function BuddyPresence({
  state,
  headline,
  speech,
  sceneMounted,
  sceneToggleLabel,
  onToggleScene,
  onControl,
}: {
  state: BuddySemanticState;
  headline: string;
  speech: BuddySceneSpeech | null;
  sceneMounted: boolean;
  sceneToggleLabel: string;
  onToggleScene: () => void;
  onControl: (control: BuddyControl) => void | Promise<void>;
}) {
  const [pendingId, setPendingId] = useState<string | null>(null);
  const [textOnly, setTextOnly] = useBuddyTextPresence();
  const [failed, setFailed] = useState(false);
  const request = useRef<object | null>(null);
  useEffect(() => {
    request.current = null;
    setPendingId(null);
    setFailed(false);
    return () => {
      request.current = null;
    };
  }, [speech?.id]);
  const runControl = async (control: BuddyControl) => {
    if (request.current) return;
    const token = {};
    request.current = token;
    setPendingId(control.id);
    setFailed(false);
    try {
      await onControl(control);
    } catch {
      if (request.current === token) setFailed(true);
    } finally {
      if (request.current === token) {
        request.current = null;
        setPendingId(null);
      }
    }
  };
  return (
    <section
      className={styles.presence}
      aria-label="Buddy presence"
      data-testid="buddy-presence"
    >
      <header className={styles.header}>
        {!sceneMounted && !textOnly && (
          <div
            className={styles.avatar}
            aria-hidden="true"
            data-testid="buddy-presence-avatar"
          >
            <BuddyErrorBoundary>
              <BuddyCanvas state={state} displaySize={76} />
            </BuddyErrorBoundary>
          </div>
        )}
        <div className={styles.identity}>
          <Text weight="bold">{state.name}</Text>
          <Text size="1" color="gray">
            {headline}
          </Text>
        </div>
        <Button
          size="sm"
          variant="ghost"
          onClick={onToggleScene}
          aria-expanded={sceneMounted}
        >
          {sceneToggleLabel}
        </Button>
        <Button
          size="sm"
          variant="ghost"
          aria-pressed={textOnly}
          onClick={() => setTextOnly(!textOnly)}
        >
          Text-only presence
        </Button>
      </header>
      {textOnly && (
        <Text size="1" color="gray">
          Desk animation is off on this device. Reports, decisions and optional
          Play remain available.
        </Text>
      )}
      {speech && (
        <div className={styles.speech}>
          <p>{speech.text}</p>
          <div className={styles.controls}>
            {speech.controls.map((control) => (
              <Button
                key={control.id}
                size="sm"
                variant={control.style === "primary" ? "primary" : "ghost"}
                disabled={pendingId !== null}
                onClick={() => void runControl(control)}
              >
                {control.label}
              </Button>
            ))}
          </div>
          {failed && <p role="alert">Action did not complete. Try again.</p>}
        </div>
      )}
    </section>
  );
}
