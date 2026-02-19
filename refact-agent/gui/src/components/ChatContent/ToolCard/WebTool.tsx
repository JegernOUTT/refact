import React, { useMemo } from "react";
import { GlobeIcon } from "@radix-ui/react-icons";
import { Box } from "@radix-ui/themes";
import { ToolCard, ToolStatus } from "./ToolCard";
import { useStoredOpen } from "../useStoredOpen";
import { ContextFileList } from "./ContextFileList";
import { useAppSelector } from "../../../hooks";
import { selectToolResultById } from "../../../features/Chat/Thread/selectors";
import { ChatContextFile, ToolCall } from "../../../services/refact/types";
import { ShikiCodeBlock } from "../../Markdown";
import styles from "./WebTool.module.css";

type WebToolType = "web" | "web_search";

interface WebArgs {
  url?: string;
}

interface WebSearchArgs {
  query?: string;
}

interface WebToolProps {
  toolCall: ToolCall;
  toolType: WebToolType;
  contextFiles?: ChatContextFile[];
}

function extractDomain(url: string): string {
  try {
    const parsed = new URL(url);
    return parsed.hostname;
  } catch {
    return url;
  }
}

export const WebTool: React.FC<WebToolProps> = ({
  toolCall,
  toolType,
  contextFiles,
}) => {
  const storeKey = toolCall.id ? `tc:${toolCall.id}` : undefined;
  const [isOpen, handleToggle] = useStoredOpen(storeKey);

  const maybeResult = useAppSelector((state) =>
    selectToolResultById(state, toolCall.id),
  );

  const args = useMemo((): WebArgs | WebSearchArgs => {
    try {
      return JSON.parse(toolCall.function.arguments) as WebArgs | WebSearchArgs;
    } catch {
      return {};
    }
  }, [toolCall.function.arguments]);

  const status: ToolStatus = useMemo(() => {
    if (!maybeResult) return "running";
    if (
      typeof maybeResult === "object" &&
      "tool_failed" in maybeResult &&
      maybeResult.tool_failed
    ) {
      return "error";
    }
    return "success";
  }, [maybeResult]);

  const content =
    maybeResult && typeof maybeResult.content === "string"
      ? maybeResult.content
      : null;

  const summary = useMemo(() => {
    if (toolType === "web") {
      const webArgs = args as WebArgs;
      const url = webArgs.url ?? "page";
      return (
        <>
          Fetch <span className={styles.url}>{extractDomain(url)}</span>
        </>
      );
    }

    // toolType === "web_search"
    const searchArgs = args as WebSearchArgs;
    const query = searchArgs.query ?? "query";
    return (
      <>
        Search web <span className={styles.query}>&quot;{query}&quot;</span>
      </>
    );
  }, [toolType, args]);

  return (
    <ToolCard
      icon={<GlobeIcon />}
      summary={summary}
      status={status}
      isOpen={isOpen}
      onToggle={handleToggle}
      toolCall={toolCall}
    >
      {content && (
        <Box className={styles.resultContent}>
          <ShikiCodeBlock showLineNumbers={false}>{content}</ShikiCodeBlock>
        </Box>
      )}
      {contextFiles && contextFiles.length > 0 && (
        <ContextFileList files={contextFiles} />
      )}
    </ToolCard>
  );
};

export default WebTool;
