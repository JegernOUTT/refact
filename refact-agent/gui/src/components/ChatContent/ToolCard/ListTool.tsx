import React from "react";
import { ChatContextFile, ToolCall } from "../../../services/refact/types";
import { SearchTool } from "./SearchTool";

interface ListToolProps {
  toolCall: ToolCall;
  contextFiles?: ChatContextFile[];
}

export const ListTool: React.FC<ListToolProps> = ({
  toolCall,
  contextFiles,
}) => (
  <SearchTool contextFiles={contextFiles} toolCall={toolCall} toolType="tree" />
);

export default ListTool;
