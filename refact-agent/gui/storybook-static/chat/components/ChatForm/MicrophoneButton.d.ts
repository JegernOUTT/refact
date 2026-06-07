import React from "react";
interface MicrophoneButtonProps {
    onTranscript: (text: string) => void;
    onLiveTranscript?: (text: string) => void;
    onRecordingChange?: (isRecording: boolean, isFinishing: boolean) => void;
    disabled?: boolean;
}
export interface MicrophoneButtonRef {
    toggleRecording: () => Promise<string | null>;
}
export declare const MicrophoneButton: React.ForwardRefExoticComponent<MicrophoneButtonProps & React.RefAttributes<MicrophoneButtonRef>>;
export {};
