import { FC } from "react";
import { Integration } from "../../../services/refact";
type ErrorStateProps = {
    integration: Integration;
    onDelete: (path: string) => void;
    isApplying: boolean;
    isDeletingIntegration: boolean;
};
export declare const ErrorState: FC<ErrorStateProps>;
export {};
