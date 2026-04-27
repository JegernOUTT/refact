import React, { useCallback } from "react";
import { Button } from "@radix-ui/themes";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { push } from "../Pages/pagesSlice";
import { clearActiveSpeech, selectActiveSpeech } from "./buddySlice";
import { openChatInModeAndStart } from "../Chat/Thread/actions";
import type { BuddyControl } from "./types";
import styles from "./BuddySpeechCloud.module.css";

interface Props {
  variant?: "block" | "overlay";
}

export const BuddySpeechCloud: React.FC<Props> = ({ variant = "block" }) => {
  const dispatch = useAppDispatch();
  const speech = useAppSelector(selectActiveSpeech);

  const handleControl = useCallback(
    (ctrl: BuddyControl) => {
      switch (ctrl.action) {
        case "dismiss":
          dispatch(clearActiveSpeech());
          break;
        case "open_setup":
          void dispatch(openChatInModeAndStart({ mode: "setup" }));
          dispatch(clearActiveSpeech());
          break;
        case "open_stats":
          dispatch(push({ name: "stats dashboard" }));
          dispatch(clearActiveSpeech());
          break;
        case "open_buddy":
          dispatch(push({ name: "buddy" }));
          dispatch(clearActiveSpeech());
          break;
        default:
          dispatch(clearActiveSpeech());
      }
    },
    [dispatch],
  );

  if (!speech) return null;

  const isOverlay = variant === "overlay";

  return (
    <div className={isOverlay ? styles.cloudOverlay : styles.cloud}>
      <p className={isOverlay ? styles.overlayText : styles.text}>
        {speech.text}
      </p>
      <div className={styles.controls}>
        {speech.controls.map((ctrl) => (
          <Button
            key={ctrl.id}
            size="1"
            variant={ctrl.style === "primary" ? "solid" : "soft"}
            onClick={() => handleControl(ctrl)}
          >
            {ctrl.label}
          </Button>
        ))}
        <Button
          size="1"
          variant="ghost"
          color="gray"
          onClick={() => dispatch(clearActiveSpeech())}
        >
          ✕
        </Button>
      </div>
      <div className={isOverlay ? styles.overlayTail : styles.tail} />
    </div>
  );
};
