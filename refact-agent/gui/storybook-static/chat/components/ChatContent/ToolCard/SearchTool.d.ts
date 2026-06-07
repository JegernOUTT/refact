import React from "react";
import { ChatContextFile, ToolCall } from "../../../services/refact/types";
type SearchToolType = "search_pattern" | "search_semantic" | "search_symbol_definition";
interface SearchToolProps {
    toolCall: ToolCall;
    toolType: SearchToolType;
    contextFiles?: ChatContextFile[];
}
export declare const SearchTool: React.FC<SearchToolProps>;
export default SearchTool;
