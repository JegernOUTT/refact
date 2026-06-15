import { forwardRef, useEffect, useImperativeHandle, useRef } from "react";
import classNames from "classnames";
import { Mic } from "lucide-react";
import { IconButton, Tooltip } from "../ui";
import { useVoiceInput } from "../../hooks/useVoiceInput";
import { useAppDispatch } from "../../hooks";
import { setError } from "../../features/Errors/errorsSlice";
import styles from "./MicrophoneButton.module.css";

interface MicrophoneButtonProps {
  onTranscript: (text: string) => void;
  onLiveTranscript?: (text: string) => void;
  onRecordingChange?: (isRecording: boolean, isFinishing: boolean) => void;
  disabled?: boolean;
}

export interface MicrophoneButtonRef {
  toggleRecording: () => Promise<string | null>;
}

export const MicrophoneButton = forwardRef<
  MicrophoneButtonRef,
  MicrophoneButtonProps
>(({ onTranscript, onLiveTranscript, onRecordingChange, disabled }, ref) => {
  const dispatch = useAppDispatch();
  const {
    isRecording,
    isFinishing,
    isDownloading,
    voiceEnabled,
    error,
    liveTranscript,
    toggleRecording,
  } = useVoiceInput(onTranscript);

  const prevTranscriptRef = useRef(liveTranscript);
  const prevRecordingRef = useRef(isRecording);
  const prevFinishingRef = useRef(isFinishing);

  useImperativeHandle(
    ref,
    () => ({
      toggleRecording,
    }),
    [toggleRecording],
  );

  useEffect(() => {
    if (error) {
      dispatch(setError(error));
    }
  }, [error, dispatch]);

  useEffect(() => {
    if (
      isRecording !== prevRecordingRef.current ||
      isFinishing !== prevFinishingRef.current
    ) {
      prevRecordingRef.current = isRecording;
      prevFinishingRef.current = isFinishing;
      onRecordingChange?.(isRecording, isFinishing);
    }
  }, [isRecording, isFinishing, onRecordingChange]);

  useEffect(() => {
    if (liveTranscript !== prevTranscriptRef.current) {
      prevTranscriptRef.current = liveTranscript;
      onLiveTranscript?.(liveTranscript);
    }
  }, [liveTranscript, onLiveTranscript]);

  if (!voiceEnabled) {
    return null;
  }

  const isActive = isRecording || isFinishing;

  return (
    <Tooltip>
      <Tooltip.Trigger asChild>
        <IconButton
          aria-label="Voice input"
          className={classNames(
            isActive && styles.active,
            isRecording && styles.recording,
            isFinishing && styles.finishing,
          )}
          disabled={!!disabled || isDownloading || isFinishing}
          icon={Mic}
          loading={isDownloading}
          size="sm"
          type="button"
          variant="plain"
          onClick={() => void toggleRecording()}
        />
      </Tooltip.Trigger>
      <Tooltip.Content>Voice input (Ctrl+Shift+Space)</Tooltip.Content>
    </Tooltip>
  );
});

MicrophoneButton.displayName = "MicrophoneButton";
