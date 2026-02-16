import React, { useMemo } from "react";
import { DesktopIcon } from "@radix-ui/react-icons";
import { Text } from "@radix-ui/themes";

import { ToolCard } from "./ToolCard";
import type { ToolCall } from "../../../services/refact/types";
import { ShikiCodeBlock } from "../../Markdown";
import styles from "./OpenAIResponsesTool.module.css";
import { useOpenAiResponsesToolCardState } from "./openaiResponsesToolCardState";

type Props = {
  toolCall: ToolCall;
};

export const OpenAIComputerCallTool: React.FC<Props> = ({ toolCall }) => {
  const state = useOpenAiResponsesToolCardState(toolCall);
  const args = state.parsedArgs as Record<string, unknown> | null;

  const summary = useMemo(() => {
    const action =
      args && typeof args.action === "string" ? args.action : undefined;
    return action ? (
      <>
        Computer Call: <span className={styles.inlineCode}>{action}</span>
      </>
    ) : (
      state.label
    );
  }, [args, state.label]);

  return (
    <ToolCard
      icon={<DesktopIcon />}
      summary={summary}
      status={state.status}
      isOpen={state.isOpen}
      onToggle={state.toggleOpen}
      toolCall={toolCall}
    >
      <Text size="1" color="gray">
        Raw JSON
      </Text>
      <ShikiCodeBlock showLineNumbers={false}>{state.rawJson}</ShikiCodeBlock>
    </ToolCard>
  );
};
