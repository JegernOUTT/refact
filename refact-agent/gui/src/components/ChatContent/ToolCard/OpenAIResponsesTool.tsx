import React, { useMemo } from "react";
import {
  CubeIcon,
  FileTextIcon,
  MagnifyingGlassIcon,
  LapTimerIcon,
  DesktopIcon,
  SpeakerLoudIcon,
  ImageIcon,
  ExclamationTriangleIcon,
} from "@radix-ui/react-icons";
import { Box, Flex, Text } from "@radix-ui/themes";
import { ToolCard, ToolStatus } from "./ToolCard";
import { useStoredOpen } from "../useStoredOpen";
import { useAppSelector } from "../../../hooks";
import {
  selectIsStreaming,
  selectIsWaiting,
  selectToolResultById,
} from "../../../features/Chat/Thread/selectors";
import { ToolCall } from "../../../services/refact/types";
import { ShikiCodeBlock } from "../../Markdown";
import { Markdown } from "../../Markdown";
import styles from "./OpenAIResponsesTool.module.css";
import { toolNameLabel } from "./openaiResponsesToolCardState";

type OpenAIResponsesToolProps = {
  toolCall: ToolCall;
};

function parseJsonOrNull(text: string): unknown {
  try {
    return JSON.parse(text) as unknown;
  } catch {
    return null;
  }
}

function getToolIcon(toolName: string): React.ReactNode {
  switch (toolName) {
    case "openai_web_search_call":
      return <MagnifyingGlassIcon />;
    case "openai_file_search_call":
      return <FileTextIcon />;
    case "openai_code_interpreter_call":
      return <LapTimerIcon />;
    case "openai_computer_call":
    case "openai_computer_call_output":
      return <DesktopIcon />;
    case "openai_audio":
      return <SpeakerLoudIcon />;
    case "openai_image_generation_call":
      return <ImageIcon />;
    case "openai_refusal":
      return <ExclamationTriangleIcon />;
    default:
      return <CubeIcon />;
  }
}

function extractToolSummary(toolName: string, args: unknown): React.ReactNode {
  const label = toolNameLabel(toolName);
  if (!args || typeof args !== "object") {
    return label;
  }

  // Most server tools pass the full output item JSON as arguments.
  const obj = args as Record<string, unknown>;
  const t = typeof obj.type === "string" ? obj.type : undefined;

  if (toolName === "openai_web_search_call") {
    const q = typeof obj.query === "string" ? obj.query : undefined;
    return q ? (
      <>
        Web Search: <span className={styles.inlineCode}>{q}</span>
      </>
    ) : (
      label
    );
  }

  if (toolName === "openai_file_search_call") {
    const q = typeof obj.query === "string" ? obj.query : undefined;
    return q ? (
      <>
        File Search: <span className={styles.inlineCode}>{q}</span>
      </>
    ) : (
      label
    );
  }

  if (toolName === "openai_code_interpreter_call") {
    return <>{t ? `Code Interpreter (${t})` : "Code Interpreter"}</>;
  }

  if (toolName === "openai_computer_call") {
    return <>{t ? `Computer Call (${t})` : "Computer Call"}</>;
  }

  if (toolName === "openai_computer_call_output") {
    return <>{t ? `Computer Output (${t})` : "Computer Output"}</>;
  }

  if (toolName === "openai_audio") {
    return <>{t ? `Audio (${t})` : "Audio"}</>;
  }

  if (toolName === "openai_image_generation_call") {
    return <>{t ? `Image (${t})` : "Image Generation"}</>;
  }

  if (toolName === "openai_refusal") {
    return "Refusal";
  }

  return label;
}

export const OpenAIResponsesTool: React.FC<OpenAIResponsesToolProps> = ({
  toolCall,
}) => {
  const storeKey = toolCall.id ? `tc:${toolCall.id}` : undefined;
  const [isOpen, handleToggle] = useStoredOpen(storeKey);
  const isStreaming = useAppSelector(selectIsStreaming);
  const isWaiting = useAppSelector(selectIsWaiting);

  const maybeResult = useAppSelector((state) =>
    selectToolResultById(state, toolCall.id),
  );

  const status: ToolStatus = useMemo(() => {
    if (!maybeResult && (isStreaming || isWaiting)) return "running";
    if (!maybeResult) return "running";
    if (
      typeof maybeResult === "object" &&
      "tool_failed" in maybeResult &&
      maybeResult.tool_failed
    ) {
      return "error";
    }
    return "success";
  }, [maybeResult, isStreaming, isWaiting]);

  const content =
    maybeResult && typeof maybeResult.content === "string"
      ? maybeResult.content
      : null;

  const toolName = toolCall.function.name ?? "openai";

  const parsedArgs = useMemo(
    () => parseJsonOrNull(toolCall.function.arguments),
    [toolCall.function.arguments],
  );

  const summary = useMemo(
    () => extractToolSummary(toolName, parsedArgs),
    [toolName, parsedArgs],
  );

  const rawJson = useMemo(() => {
    if (parsedArgs == null) return toolCall.function.arguments;
    try {
      return JSON.stringify(parsedArgs, null, 2);
    } catch {
      return toolCall.function.arguments;
    }
  }, [parsedArgs, toolCall.function.arguments]);

  return (
    <ToolCard
      icon={getToolIcon(toolName)}
      summary={summary}
      status={status}
      isOpen={isOpen}
      onToggle={handleToggle}
      toolCall={toolCall}
    >
      <Box className={styles.content}>
        {content ? <Markdown>{content}</Markdown> : null}

        {parsedArgs != null && typeof parsedArgs === "object" && (
          <Box mb="2">{renderOpenAiResponsesPayload(toolName, parsedArgs)}</Box>
        )}

        <Text size="1" color="gray">
          Raw JSON
        </Text>
        <ShikiCodeBlock showLineNumbers={false}>{rawJson}</ShikiCodeBlock>
      </Box>
    </ToolCard>
  );
};

function renderOpenAiResponsesPayload(
  toolName: string,
  args: unknown,
): React.ReactNode {
  const obj = args as Record<string, unknown>;

  // Web search: show results list (if present)
  if (toolName === "openai_web_search_call") {
    const results = Array.isArray(obj.results)
      ? (obj.results as unknown[])
      : [];
    if (results.length > 0) {
      return (
        <Box>
          <Text size="1" color="gray">
            Results ({results.length})
          </Text>
          <Box className={styles.resultList}>
            {results.slice(0, 20).map((r, idx) => {
              const rr = r as Record<string, unknown>;
              const title =
                typeof rr.title === "string" ? rr.title : "(no title)";
              const url = typeof rr.url === "string" ? rr.url : "";
              const snippet =
                typeof rr.snippet === "string"
                  ? rr.snippet
                  : typeof rr.description === "string"
                    ? rr.description
                    : "";
              return (
                <Box key={idx} className={styles.resultItem}>
                  <Flex direction="column" gap="1">
                    <Text size="2" weight="medium">
                      {title}
                    </Text>
                    {url && (
                      <Text size="1" color="gray" className={styles.inlineCode}>
                        {url}
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
      );
    }
  }

  // File search: show matches (if present)
  if (toolName === "openai_file_search_call") {
    const results = Array.isArray(obj.results)
      ? (obj.results as unknown[])
      : [];
    if (results.length > 0) {
      return (
        <Box>
          <Text size="1" color="gray">
            Matches ({results.length})
          </Text>
          <Box className={styles.resultList}>
            {results.slice(0, 50).map((r, idx) => {
              const rr = r as Record<string, unknown>;
              const filename =
                typeof rr.filename === "string"
                  ? rr.filename
                  : typeof rr.file_name === "string"
                    ? rr.file_name
                    : "(file)";
              const text =
                typeof rr.text === "string"
                  ? rr.text
                  : typeof rr.content === "string"
                    ? rr.content
                    : "";
              return (
                <Box key={idx} className={styles.resultItem}>
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
      );
    }
  }

  // Code interpreter outputs: preserve JSON, but try to show text outputs.
  if (toolName === "openai_code_interpreter_call") {
    const outputs = Array.isArray(obj.outputs)
      ? (obj.outputs as unknown[])
      : [];
    if (outputs.length > 0) {
      const textOutputs: string[] = [];
      for (const out of outputs) {
        const oo = out as Record<string, unknown>;
        if (typeof oo.text === "string") textOutputs.push(oo.text);
      }
      if (textOutputs.length > 0) {
        return (
          <Box className={styles.codeBox}>
            <ShikiCodeBlock showLineNumbers={false}>
              {textOutputs.join("\n\n")}
            </ShikiCodeBlock>
          </Box>
        );
      }
    }
  }

  // Computer call output can include images; keep JSON for now (images will be rendered in result content if present there)
  // Audio and refusal: keep JSON; assistant UI already handles transcripts/refusal text if surfaced.

  return null;
}
