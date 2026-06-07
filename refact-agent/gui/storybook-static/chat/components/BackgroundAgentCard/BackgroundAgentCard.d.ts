import { JSX } from 'react/jsx-runtime';
import type { BackgroundAgentSummary } from "../../services/refact/types";
export interface BackgroundAgentCardProps {
    agent: BackgroundAgentSummary;
    onOpenTrajectory?: (childChatId: string) => void;
}
export declare const BackgroundAgentCard: ({ agent, onOpenTrajectory, }: BackgroundAgentCardProps) => JSX.Element;
