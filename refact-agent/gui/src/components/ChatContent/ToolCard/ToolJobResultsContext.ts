import { createContext } from "react";
import {
  EMPTY_TOOL_JOB_RESULTS,
  type ToolJobResultsByToolId,
} from "../ChatContentDisplayItems";

export const ToolJobResultsContext = createContext<ToolJobResultsByToolId>(
  EMPTY_TOOL_JOB_RESULTS,
);
