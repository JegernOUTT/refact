import { FC } from "react";
type CapabilityBadgeProps = {
    name: string;
    enabled: boolean;
    displayValue?: string | null;
    onClick?: () => void;
    interactive?: boolean;
};
/**
 * Reusable component for model capability badges
 */
export declare const CapabilityBadge: FC<CapabilityBadgeProps>;
export {};
