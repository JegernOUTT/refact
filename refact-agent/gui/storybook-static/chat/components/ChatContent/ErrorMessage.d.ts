import React from "react";
import type { ErrorMessage } from "../../services/refact/types";
export type ErrorMessageCardProps = {
    errors: ErrorMessage[];
};
export declare const ErrorMessageCard: React.FC<ErrorMessageCardProps>;
