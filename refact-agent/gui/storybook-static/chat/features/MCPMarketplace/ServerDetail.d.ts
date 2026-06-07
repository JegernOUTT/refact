import React from "react";
import type { MCPServer } from "../../services/refact/mcpMarketplace";
type ServerDetailProps = {
    server: MCPServer;
    onBack: () => void;
};
export declare const ServerDetail: React.FC<ServerDetailProps>;
export {};
