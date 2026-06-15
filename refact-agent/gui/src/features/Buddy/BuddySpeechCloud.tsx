import React, { useCallback } from "react";
import { Badge, Button } from "../../components/ui";
import { useAppDispatch, useAppSelector } from "../../hooks";
import {
  clearActiveSpeech,
  selectActiveSpeech,
  selectBuddyDiagnostics,
} from "./buddySlice";
import { executeBuddyAction } from "./executeBuddyAction";
import type { BuddyControl, BuddySpeechItem } from "./types";
import styles from "./BuddySpeechCloud.module.css";

type SpeechWithIntent = Pick<
  BuddySpeechItem,
  "text" | "controls" | "chat_id"
> & {
  speech_intent?: string;
};

interface Props {
  variant?: "block" | "overlay";
  tailSide?: "bottom" | "right";
  speech?: SpeechWithIntent;
  onControl?: (ctrl: BuddyControl) => void | Promise<void>;
}

export const BuddySpeechCloud: React.FC<Props> = ({
  variant = "block",
  tailSide = "bottom",
  speech: speechProp,
  onControl,
}) => {
  const dispatch = useAppDispatch();
  const activeSpeech = useAppSelector(selectActiveSpeech);
  const diagnostics = useAppSelector(selectBuddyDiagnostics);

  const speech = speechProp ?? activeSpeech;
  const speechDiagnostic = speech?.chat_id
    ? diagnostics.find((diag) => diag.chat_id === speech.chat_id)
    : undefined;

  const internalHandleControl = useCallback(
    async (ctrl: BuddyControl) => {
      if (!speech) return;
      await executeBuddyAction(ctrl, dispatch, {
        triggerText: speech.text,
        triggerSource: "runtime",
        sourceChatId: speech.chat_id,
        diagnostic: speechDiagnostic,
      });
    },
    [dispatch, speech, speechDiagnostic],
  );

  const handleControl = onControl ?? internalHandleControl;

  if (!speech) return null;

  const isOverlay = variant === "overlay";

  return (
    <div className={isOverlay ? styles.cloudOverlay : styles.cloud}>
      {speech.speech_intent && (
        <Badge size="1" variant="soft" className={styles.intentBadge}>
          {speech.speech_intent}
        </Badge>
      )}
      <p className={isOverlay ? styles.overlayText : styles.text}>
        {speech.text}
      </p>
      <div className={styles.controls}>
        {speech.controls.map((ctrl) => (
          <Button
            key={ctrl.id}
            size="1"
            variant={ctrl.style === "primary" ? "solid" : "soft"}
            onClick={() => void handleControl(ctrl)}
          >
            {ctrl.label}
          </Button>
        ))}
        {!onControl && (
          <Button
            size="1"
            variant="ghost"
            color="gray"
            onClick={() => dispatch(clearActiveSpeech())}
          >
            ✕
          </Button>
        )}
      </div>
      {tailSide === "right" ? (
        <div className={styles.tailRight} />
      ) : isOverlay ? (
        <div className={styles.overlayTail} />
      ) : (
        <div className={styles.tail} />
      )}
    </div>
  );
};
