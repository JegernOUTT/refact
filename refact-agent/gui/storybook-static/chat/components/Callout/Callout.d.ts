import React from "react";
import { Callout as RadixCallout } from "@radix-ui/themes";
type RadixCalloutProps = React.ComponentProps<typeof RadixCallout.Root>;
export type CalloutProps = Omit<RadixCalloutProps, "onClick"> & {
    type: "info" | "error" | "warning";
    onClick?: () => void;
    timeout?: number | null;
    preventRetry?: boolean;
    preventClose?: boolean;
    hex?: string;
    message?: string | string[];
};
export declare const Callout: React.FC<CalloutProps>;
export declare const ErrorCallout: React.FC<Omit<CalloutProps, "type">>;
export declare const InformationCallout: React.FC<Omit<CalloutProps, "type">>;
export declare const DiffWarningCallout: React.FC<Omit<CalloutProps, "type">>;
export declare const CalloutFromTop: React.FC<RadixCalloutProps & {
    children?: React.ReactNode;
}>;
export {};
