import React from "react";
type Props = {
    children?: React.ReactNode;
    showThreadReportPanel?: boolean;
};
type State = {
    failed: boolean;
};
export declare class BuddyErrorBoundary extends React.Component<Props, State> {
    state: State;
    static getDerivedStateFromError(): State;
    componentDidCatch(error: Error, errorInfo: React.ErrorInfo): void;
    render(): React.ReactNode;
}
export declare function withBuddyErrorReport<T>(fn: () => T, args: {
    source: "react_root_render" | "react_recoverable";
    sourceFile: string;
    toolName: string;
}): T;
export {};
