import { FileText } from "lucide-react";
import React, { useMemo } from "react";
import { Box, Text } from "@radix-ui/themes";

import { ToolCard } from "./ToolCard";
import type { ToolCall } from "../../../services/refact/types";
import { ShikiCodeBlock } from "../../Markdown";
import styles from "./OpenAIResponsesTool.module.css";
import { useOpenAiResponsesToolCardState } from "./openaiResponsesToolCardState";

type Props = {
  toolCall: ToolCall;
};

type FileSearchResult = {
  filename?: string;
  file_name?: string;
  text?: string;
  content?: string;
};

export const OpenAIFileSearchCallTool: React.FC<Props> = ({ toolCall }) => {
  const state = useOpenAiResponsesToolCardState(toolCall);

  const args = state.parsedArgs as Record<string, unknown> | null;
  const query = args && typeof args.query === "string" ? args.query : undefined;

  const results = useMemo(() => {
    if (!args) return [] as FileSearchResult[];
    if (!Array.isArray(args.results)) return [] as FileSearchResult[];
    return (args.results as unknown[])
      .map((r) => (typeof r === "object" && r ? (r as FileSearchResult) : {}))
      .slice(0, 200);
  }, [args]);

  const summary = query ? (
    <>
      File Search: <span className={styles.inlineCode}>{query}</span>
    </>
  ) : (
    state.label
  );

  return (
    <ToolCard
      icon={<FileText />}
      summary={summary}
      status={state.status}
      isOpen={state.isOpen}
      onToggle={state.toggleOpen}
      toolCall={toolCall}
    >
      {results.length > 0 && (
        <Box>
          <Text size="1" color="gray">
            Matches ({results.length})
          </Text>
          <Box className={`${styles.resultList} rf-stagger`}>
            {results.map((r, idx) => {
              const filename = r.filename ?? r.file_name ?? "(file)";
              const text = r.text ?? r.content ?? "";
              return (
                <Box
                  key={idx}
                  className={`${styles.resultItem} rf-enter-rise rf-pressable`}
                >
                  <Text size="2" weight="medium" className={styles.inlineCode}>
                    {filename}
                  </Text>
                  {text && (
                    <Box mt="1" className={styles.codeBox}>
                      <ShikiCodeBlock showLineNumbers={false}>
                        {text}
                      </ShikiCodeBlock>
                    </Box>
                  )}
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
