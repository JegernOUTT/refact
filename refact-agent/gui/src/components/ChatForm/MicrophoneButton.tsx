import React, {
  useEffect,
  useRef,
  useImperativeHandle,
  forwardRef,
} from "react";
import { Spinner, HoverCard, Text } from "@radix-ui/themes";
import classNames from "classnames";
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
    <HoverCard.Root>
      <HoverCard.Trigger>
        <button
          type="button"
          disabled={!!disabled || isDownloading || isFinishing}
          onClick={() => void toggleRecording()}
          className={classNames(
            styles.iconButton,
            isActive && styles.active,
            isRecording && styles.recording,
            isFinishing && styles.finishing,
          )}
          aria-label="Voice input"
        >
          {isDownloading ? <Spinner size="1" /> : <MicrophoneIcon />}
        </button>
      </HoverCard.Trigger>
      <HoverCard.Content size="1" side="top">
        <Text as="p" size="2">
          Voice input (Ctrl+Shift+Space)
        </Text>
      </HoverCard.Content>
    </HoverCard.Root>
  );
});

MicrophoneButton.displayName = "MicrophoneButton";

const MicrophoneIcon: React.FC = () => (
  <svg
    width="15"
    height="15"
    viewBox="0 0 15 15"
    fill="none"
    xmlns="http://www.w3.org/2000/svg"
  >
    <path
      d="M7.5 1C6.11929 1 5 2.11929 5 3.5V7.5C5 8.88071 6.11929 10 7.5 10C8.88071 10 10 8.88071 10 7.5V3.5C10 2.11929 8.88071 1 7.5 1Z"
      fill="currentColor"
    />
    <path
      d="M3 6.5C3.27614 6.5 3.5 6.72386 3.5 7V7.5C3.5 9.70914 5.29086 11.5 7.5 11.5C9.70914 11.5 11.5 9.70914 11.5 7.5V7C11.5 6.72386 11.7239 6.5 12 6.5C12.2761 6.5 12.5 6.72386 12.5 7V7.5C12.5 10.0376 10.5376 12.1 8 12.4649V14H10C10.2761 14 10.5 14.2239 10.5 14.5C10.5 14.7761 10.2761 15 10 15H5C4.72386 15 4.5 14.7761 4.5 14.5C4.5 14.2239 4.72386 14 5 14H7V12.4649C4.46243 12.1 2.5 10.0376 2.5 7.5V7C2.5 6.72386 2.72386 6.5 3 6.5Z"
      fill="currentColor"
    />
  </svg>
);
