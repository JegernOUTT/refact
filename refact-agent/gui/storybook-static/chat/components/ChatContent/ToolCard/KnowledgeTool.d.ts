import React from "react";
import { ChatContextFile, ToolCall } from "../../../services/refact/types";
type KnowledgeToolType = "knowledge" | "create_knowledge" | "trajectories" | "search_trajectories";
interface KnowledgeToolProps {
    toolCall: ToolCall;
    toolType: KnowledgeToolType;
    contextFiles?: ChatContextFile[];
}
export declare const KnowledgeTool: React.FC<KnowledgeToolProps>;
export default KnowledgeTool;
