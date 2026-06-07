import React from "react";
import type { BuddyControl, BuddySpeechItem } from "./types";
type SpeechWithIntent = Pick<BuddySpeechItem, "text" | "controls" | "chat_id"> & {
    speech_intent?: string;
};
interface Props {
    variant?: "block" | "overlay";
    tailSide?: "bottom" | "right";
    speech?: SpeechWithIntent;
    onControl?: (ctrl: BuddyControl) => void | Promise<void>;
}
export declare const BuddySpeechCloud: React.FC<Props>;
export {};
