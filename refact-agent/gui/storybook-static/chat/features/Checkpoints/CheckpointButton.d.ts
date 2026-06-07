import { JSX } from 'react/jsx-runtime';
import { Checkpoint } from "./types";
type CheckpointButtonProps = {
    checkpoints: Checkpoint[] | null;
    messageIndex: number;
};
export declare const CheckpointButton: ({ checkpoints, messageIndex, }: CheckpointButtonProps) => JSX.Element;
export {};
