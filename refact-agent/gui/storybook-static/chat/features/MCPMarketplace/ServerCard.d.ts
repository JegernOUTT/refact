import React from "react";
import type { MCPServer } from "../../services/refact/mcpMarketplace";
type ServerCardProps = {
    server: MCPServer;
    isInstalled: boolean;
    installedConfigPath?: string;
    onInstall: (server: MCPServer) => void;
    onViewDetail: (server: MCPServer) => void;
    onConfigure?: (configPath: string) => void;
    isInstalling: boolean;
    sourceLabel?: string;
};
export declare const ServerCard: React.FC<ServerCardProps>;
export {};
