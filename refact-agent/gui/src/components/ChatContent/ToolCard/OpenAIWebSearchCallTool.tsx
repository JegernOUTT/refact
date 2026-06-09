import { Search } from "lucide-react";
import React, { useMemo } from "react";
import { Box, Flex, Link, Text } from "@radix-ui/themes";

import { ToolCard } from "./ToolCard";
import type { ToolCall } from "../../../services/refact/types";
import { ShikiCodeBlock } from "../../Markdown";
import styles from "./OpenAIResponsesTool.module.css";
import { useOpenAiResponsesToolCardState } from "./openaiResponsesToolCardState";

type Props = {
  toolCall: ToolCall;
};

type WebSearchResult = {
  url?: string;
  title?: string;
  snippet?: string;
  description?: string;
};

function isSafeHttpUrl(url: string): boolean {
  try {
    const parsed = new URL(url);
    return parsed.protocol === "http:" || parsed.protocol === "https:";
  } catch {
    return false;
  }
}

export const OpenAIWebSearchCallTool: React.FC<Props> = ({ toolCall }) => {
  const state = useOpenAiResponsesToolCardState(toolCall);

  const args = state.parsedArgs as Record<string, unknown> | null;
  const query = args && typeof args.query === "string" ? args.query : undefined;

  const results = useMemo(() => {
    if (!args) return [] as WebSearchResult[];
    if (!Array.isArray(args.results)) return [] as WebSearchResult[];
    return (args.results as unknown[])
      .map((r) => (typeof r === "object" && r ? (r as WebSearchResult) : {}))
      .slice(0, 50);
  }, [args]);

  const summary = query ? (
    <>
      Web Search: <span className={styles.inlineCode}>{query}</span>
    </>
  ) : (
    state.label
  );

  return (
    <ToolCard
      icon={<Search />}
      summary={summary}
      status={state.status}
      isOpen={state.isOpen}
      onToggle={state.toggleOpen}
      toolCall={toolCall}
    >
      {results.length > 0 && (
        <Box>
          <Text size="1" color="gray">
            Results ({results.length})
          </Text>
          <Box className={`${styles.resultList} rf-stagger`}>
            {results.map((r, idx) => {
              const title = r.title ?? "(no title)";
              const url = r.url ?? "";
              const safeUrl = url && isSafeHttpUrl(url) ? url : "";
              const snippet = r.snippet ?? r.description ?? "";
              return (
                <Box key={idx} className={`${styles.resultItem} rf-enter-rise rf-pressable`}>
                  <Flex direction="column" gap="1">
                    {safeUrl ? (
                      <Link
                        href={safeUrl}
                        target="_blank"
                        rel="noopener noreferrer"
                        size="2"
                      >
                        {title}
                      </Link>
                    ) : (
                      <Text size="2" weight="medium">
                        {title}
                      </Text>
                    )}
                    {safeUrl && (
                      <Text size="1" color="gray" className={styles.inlineCode}>
                        {safeUrl}
                      </Text>
                    )}
                    {snippet && (
                      <Text size="1" color="gray">
                        {snippet}
                      </Text>
                    )}
                  </Flex>
                </Box>
              );
            })}
          </Box>
        </Box>
      )}

      <Text size="1" color="gray">
        Raw JSON
      </Text>
      <Box className={styles.rawJson}>
        <ShikiCodeBlock showLineNumbers={false}>{state.rawJson}</ShikiCodeBlock>
      </Box>
    </ToolCard>
  );
};
