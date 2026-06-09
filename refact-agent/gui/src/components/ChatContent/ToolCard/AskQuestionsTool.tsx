import { CircleHelp, CircleCheck } from "lucide-react";
import React, {
  useMemo,
  useState,
  useCallback,
  useEffect,
  useRef,
} from "react";
import { Box, Flex, Text } from "@radix-ui/themes";
import { ToolCard, ToolStatus } from "./ToolCard";
import { Button, FieldTextarea, SegmentedControl, Switch } from "../../ui";
import { useStoredOpen } from "../useStoredOpen";
import { Markdown } from "../../Markdown";
import { useAppSelector, useChatActions } from "../../../hooks";
import {
  selectToolResultById,
  selectMessages,
} from "../../../features/Chat/Thread/selectors";
import {
  ToolCall,
  isUserMessage,
  isToolMessage,
} from "../../../services/refact/types";
import {
  clearAskQuestionsDraft,
  loadAskQuestionsDraft,
  saveAskQuestionsDraft,
  type AskQuestionsDraftValue,
} from "../../../utils/chatUiPersistence";
import styles from "./AskQuestionsTool.module.css";

interface QuestionItem {
  id: string;
  type: "yes_no" | "single_select" | "multi_select" | "free_text";
  text: string;
  options?: string[];
}

interface AskQuestionsResult {
  type: "ask_questions";
  tool_call_id: string;
  questions: QuestionItem[];
}

interface AskQuestionsToolProps {
  toolCall: ToolCall;
}

function formatAnswers(
  marker: string,
  questions: QuestionItem[],
  answers: Record<string, string | string[]>,
  additional: string,
): string {
  const lines = [marker];

  for (const q of questions) {
    const answer = answers[q.id];
    lines.push(`> [${q.id}] ${q.text}`);
    if (Array.isArray(answer)) {
      lines.push(answer.length > 0 ? answer.join(", ") : "(no selection)");
    } else if (answer && answer.includes("\n")) {
      lines.push("```");
      lines.push(answer);
      lines.push("```");
    } else {
      lines.push(answer || "(no answer)");
    }
    lines.push("");
  }

  if (additional.trim()) {
    lines.push("> [__additional__] Additional comments");
    if (additional.includes("\n")) {
      lines.push("```");
      lines.push(additional.trim());
      lines.push("```");
    } else {
      lines.push(additional.trim());
    }
  }

  return lines.join("\n").trim();
}

function parseAnswersFromMessage(
  content: string,
  questions: QuestionItem[],
): Record<string, string> | null {
  const result: Record<string, string> = {};
  const idSet = new Set(questions.map((q) => q.id));
  idSet.add("__additional__");

  const regex = /^> \[([^\]]+)\]/gm;
  let match;
  const positions: { id: string; start: number }[] = [];

  while ((match = regex.exec(content)) !== null) {
    if (idSet.has(match[1])) {
      positions.push({ id: match[1], start: match.index });
    }
  }

  for (let i = 0; i < positions.length; i++) {
    const { id, start } = positions[i];
    const lineEnd = content.indexOf("\n", start);
    if (lineEnd === -1) continue; // Guard against missing newline
    const answerStart = lineEnd + 1;
    const answerEnd =
      i + 1 < positions.length ? positions[i + 1].start : content.length;

    let answer = content.slice(answerStart, answerEnd).trim();
    if (answer.startsWith("```") && answer.includes("```", 3)) {
      const codeStart = answer.indexOf("\n") + 1;
      const codeEnd = answer.lastIndexOf("```");
      answer = answer.slice(codeStart, codeEnd).trim();
    }
    if (answer) {
      result[id] = answer;
    }
  }

  return Object.keys(result).length > 0 ? result : null;
}

const QuestionWidget: React.FC<{
  question: QuestionItem;
  value: string | string[];
  onChange: (val: string | string[]) => void;
}> = ({ question, value, onChange }) => {
  switch (question.type) {
    case "yes_no":
      return (
        <Box className={styles.questionItem}>
          <Box mb="2">
            <Markdown>{question.text}</Markdown>
          </Box>
          <SegmentedControl
            name={question.id}
            options={[
              { value: "Yes", label: "Yes" },
              { value: "No", label: "No" },
            ]}
            value={typeof value === "string" ? value : ""}
            onValueChange={onChange}
          />
        </Box>
      );

    case "single_select":
      return (
        <Box className={styles.questionItem}>
          <Box mb="2">
            <Markdown>{question.text}</Markdown>
          </Box>
          <Flex
            aria-label={question.text}
            className={styles.singleSelectGroup}
            direction="column"
            gap="2"
            role="radiogroup"
          >
            {question.options?.map((opt) => {
              const checked = typeof value === "string" && value === opt;
              const optionClassName = checked
                ? `${styles.singleSelectOption} ${styles.singleSelectOptionSelected}`
                : styles.singleSelectOption;
              return (
                <label key={opt} className={optionClassName}>
                  <input
                    className={styles.singleSelectInput}
                    type="radio"
                    name={question.id}
                    value={opt}
                    checked={checked}
                    onChange={() => onChange(opt)}
                  />
                  <span className={styles.singleSelectIndicator} />
                  <span className={styles.singleSelectLabel}>{opt}</span>
                </label>
              );
            })}
          </Flex>
        </Box>
      );

    case "multi_select":
      return (
        <Box className={styles.questionItem}>
          <Box mb="2">
            <Markdown>{question.text}</Markdown>
          </Box>
          <Flex direction="column" gap="2">
            {question.options?.map((opt) => {
              const checked = Array.isArray(value) && value.includes(opt);
              return (
                <Switch
                  key={opt}
                  checked={checked}
                  label={opt}
                  onCheckedChange={(nextChecked) => {
                    const current = Array.isArray(value) ? value : [];
                    if (nextChecked) {
                      onChange([...current, opt]);
                      return;
                    }
                    onChange(current.filter((v) => v !== opt));
                  }}
                />
              );
            })}
          </Flex>
        </Box>
      );

    case "free_text":
      return (
        <Box className={styles.questionItem}>
          <Box mb="2">
            <Markdown>{question.text}</Markdown>
          </Box>
          <FieldTextarea
            value={typeof value === "string" ? value : ""}
            onChange={onChange}
            placeholder="Type your answer..."
          />
        </Box>
      );

    default:
      return null;
  }
};

export const AskQuestionsTool: React.FC<AskQuestionsToolProps> = ({
  toolCall,
}) => {
  const storeKey = toolCall.id ? `tc:${toolCall.id}` : undefined;
  const [isOpen, handleToggle, setIsOpen] = useStoredOpen(storeKey, true);
  const [answers, setAnswers] = useState<
    Record<string, AskQuestionsDraftValue>
  >(() => loadAskQuestionsDraft(toolCall.id)?.answers ?? {});
  const [additionalText, setAdditionalText] = useState(
    () => loadAskQuestionsDraft(toolCall.id)?.additionalText ?? "",
  );
  const hasCollapsedManualRef = useRef(false);

  const { submit } = useChatActions();

  const maybeResult = useAppSelector((state) =>
    selectToolResultById(state, toolCall.id),
  );

  const messages = useAppSelector(selectMessages);

  const data = useMemo((): AskQuestionsResult | null => {
    if (!maybeResult || typeof maybeResult.content !== "string") return null;
    try {
      return JSON.parse(maybeResult.content) as AskQuestionsResult;
    } catch {
      return null;
    }
  }, [maybeResult]);

  const marker = `[QA:${toolCall.id}]`;

  const nextUserMessage = useMemo(() => {
    if (!maybeResult) return null;

    let foundToolResult = false;
    for (const msg of messages) {
      if (isToolMessage(msg) && msg.tool_call_id === toolCall.id) {
        foundToolResult = true;
        continue;
      }
      if (foundToolResult && isUserMessage(msg)) {
        return msg;
      }
    }
    return null;
  }, [messages, maybeResult, toolCall.id]);

  const getContentText = useCallback((content: unknown): string => {
    if (typeof content === "string") return content;
    if (!Array.isArray(content)) return "";
    for (const item of content) {
      if (typeof item === "object" && item !== null) {
        const obj = item as Record<string, unknown>;
        if (obj.type === "text" && typeof obj.text === "string") {
          return obj.text;
        }
        if (obj.m_type === "text" && typeof obj.m_content === "string") {
          return obj.m_content;
        }
      }
    }
    return "";
  }, []);

  const answeredViaForm = useMemo(() => {
    if (!nextUserMessage) return false;
    const content = getContentText(nextUserMessage.content);
    return content.startsWith(marker);
  }, [nextUserMessage, marker, getContentText]);

  const parsedAnswers = useMemo(() => {
    if (!answeredViaForm || !nextUserMessage || !data) return null;
    const content = getContentText(nextUserMessage.content);
    return parseAnswersFromMessage(content, data.questions);
  }, [answeredViaForm, nextUserMessage, data, getContentText]);

  const status: ToolStatus = useMemo(() => {
    if (!maybeResult) return "running";
    if (maybeResult.tool_failed) return "error";
    if (!nextUserMessage) return "running";
    return "success";
  }, [maybeResult, nextUserMessage]);

  useEffect(() => {
    if (nextUserMessage && !answeredViaForm && !hasCollapsedManualRef.current) {
      hasCollapsedManualRef.current = true;
      setIsOpen(false);
    }
  }, [nextUserMessage, answeredViaForm, setIsOpen]);

  const hasNextMessage = !!nextUserMessage;

  useEffect(() => {
    if (hasNextMessage) {
      clearAskQuestionsDraft(toolCall.id);
      return;
    }

    const hasAnswers = Object.keys(answers).length > 0;
    if (!hasAnswers && additionalText.trim().length === 0) {
      clearAskQuestionsDraft(toolCall.id);
      return;
    }

    saveAskQuestionsDraft(toolCall.id, answers, additionalText);
  }, [additionalText, answers, hasNextMessage, toolCall.id]);

  const handleSubmit = useCallback(() => {
    if (!data) return;
    const formatted = formatAnswers(
      marker,
      data.questions,
      answers,
      additionalText,
    );
    void submit(formatted);
  }, [data, marker, answers, additionalText, submit]);

  if (!hasNextMessage && data) {
    return (
      <ToolCard
        icon={<CircleHelp />}
        summary="Questions for you"
        status={status}
        isOpen={isOpen}
        onToggle={handleToggle}
        toolCall={toolCall}
      >
        <Box className={`${styles.content} rf-enter-rise`}>
          <Flex direction="column" gap="3">
            {data.questions.map((q) => (
              <QuestionWidget
                key={q.id}
                question={q}
                value={answers[q.id] || (q.type === "multi_select" ? [] : "")}
                onChange={(val) =>
                  setAnswers((prev) => ({ ...prev, [q.id]: val }))
                }
              />
            ))}

            <Box className={styles.questionItem}>
              <Text size="1" color="gray" mb="1" as="p">
                Additional comments (optional)
              </Text>
              <FieldTextarea
                value={additionalText}
                onChange={setAdditionalText}
                placeholder="Add any extra context..."
              />
            </Box>

            <Button onClick={handleSubmit}>Submit Answers</Button>
          </Flex>
        </Box>
      </ToolCard>
    );
  }

  if (answeredViaForm && data && parsedAnswers) {
    return (
      <ToolCard
        icon={<CircleCheck />}
        summary="Questions answered"
        status="success"
        isOpen={isOpen}
        onToggle={handleToggle}
        toolCall={toolCall}
      >
        <Box className={`${styles.content} rf-enter-rise`}>
          <Flex direction="column" gap="2">
            {data.questions.map((q) => (
              <Box key={q.id}>
                <Markdown>{q.text}</Markdown>
                <Text color="gray" size="2" ml="2">
                  → {parsedAnswers[q.id] || "(no answer)"}
                </Text>
              </Box>
            ))}
            {parsedAnswers.__additional__ && (
              <Box mt="2">
                <Text className={styles.additionalAnswer} size="2" color="gray">
                  {parsedAnswers.__additional__}
                </Text>
              </Box>
            )}
          </Flex>
        </Box>
      </ToolCard>
    );
  }

  return (
    <ToolCard
      icon={<CircleHelp />}
      summary="Questions (answered manually)"
      status="success"
      isOpen={isOpen}
      onToggle={handleToggle}
      toolCall={toolCall}
    >
      {data && (
        <Box className={`${styles.content} rf-enter-rise`}>
          <Flex direction="column" gap="1">
            {data.questions.map((q) => (
              <Box key={q.id}>
                <Markdown>{`• ${q.text}`}</Markdown>
              </Box>
            ))}
          </Flex>
        </Box>
      )}
    </ToolCard>
  );
};

export default AskQuestionsTool;
