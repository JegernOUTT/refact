import React from "react";
type ConnectionStatusValue = string | Record<string, unknown>;
type MCPConnectionStatusProps = {
    status: ConnectionStatusValue;
    onReconnect: () => void;
    isReconnecting: boolean;
};
export declare const MCPConnectionStatus: React.FC<MCPConnectionStatusProps>;
export {};
