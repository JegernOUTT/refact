import React, { useMemo, useState, useCallback } from "react";
import { GlobeIcon } from "@radix-ui/react-icons";
import { Box } from "@radix-ui/themes";
import { ToolCard, ToolStatus } from "./ToolCard";
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

function truncate(str: string, maxLen: number): string {
  if (str.length <= maxLen) return str;
  return str.slice(0, maxLen - 1) + "…";
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
  const [isOpen, setIsOpen] = useState(false);

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

  const handleToggle = useCallback(() => {
    setIsOpen((prev) => !prev);
  }, []);

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
        Search web{" "}
        <span className={styles.query}>&quot;{truncate(query, 25)}&quot;</span>
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
