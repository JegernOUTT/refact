import { JSX } from 'react/jsx-runtime';
export type AgentCapabilitiesProps = {
    trajectoryOpen?: boolean;
    onTrajectoryOpenChange?: (open: boolean) => void;
};
export declare const AgentCapabilities: ({ trajectoryOpen, onTrajectoryOpenChange, }: AgentCapabilitiesProps) => JSX.Element | null;
