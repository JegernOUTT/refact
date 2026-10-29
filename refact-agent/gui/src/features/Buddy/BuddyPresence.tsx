import React, { useEffect, useId, useRef, useState } from "react";
import { Gamepad2, ToggleLeft, ToggleRight } from "lucide-react";
import classNames from "classnames";
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
  vibe,
  speech,
  sceneMounted,
  sceneToggleLabel,
  onToggleScene,
  onControl,
  primaryAction,
}: {
  state: BuddySemanticState;
  headline: string;
  vibe?: string | null;
  speech: BuddySceneSpeech | null;
  sceneMounted: boolean;
  sceneToggleLabel: string;
  onToggleScene: () => void;
  onControl: (control: BuddyControl) => void | Promise<void>;
  primaryAction?: React.ReactNode;
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
  const nameId = useId();
  const trait = vibe?.trim();
  const mood = headline.trim();
  return (
    <section
      className={styles.presence}
      aria-labelledby={nameId}
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
          <Text
            as="h2"
            size="4"
            weight="bold"
            id={nameId}
            className={styles.name}
          >
            {state.name}
          </Text>
          {trait && (
            <Text
              size="1"
              color="gray"
              className={styles.vibe}
              data-testid="buddy-presence-vibe"
            >
              {trait}
            </Text>
          )}
        </div>
      </header>
      {mood && (
        <Text
          as="p"
          size="2"
          className={styles.mood}
          data-testid="buddy-presence-mood"
        >
          {mood}
        </Text>
      )}
      {speech && (
        <div className={styles.speech} data-testid="buddy-presence-speech">
          <Text as="p" size="2" className={styles.speechText}>
            {speech.text}
          </Text>
          {speech.controls.length > 0 && (
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
          )}
          {failed && (
            <Text as="p" size="1" role="alert" className={styles.alert}>
              Action did not complete. Try again.
            </Text>
          )}
        </div>
      )}
      <div className={styles.actions}>
        {primaryAction}
        <Button
          size="sm"
          variant="ghost"
          leftIcon={Gamepad2}
          onClick={onToggleScene}
        >
          {sceneToggleLabel}
        </Button>
        <Button
          size="sm"
          variant="ghost"
          leftIcon={textOnly ? ToggleRight : ToggleLeft}
          aria-pressed={textOnly}
          className={classNames(styles.toggle, textOnly && styles.toggleOn)}
          onClick={() => setTextOnly(!textOnly)}
        >
          Text-only presence
        </Button>
      </div>
    </section>
  );
}
