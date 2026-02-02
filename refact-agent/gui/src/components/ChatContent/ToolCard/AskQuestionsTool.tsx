import React, {
  useMemo,
  useState,
  useCallback,
  useEffect,
  useRef,
} from "react";
import {
  QuestionMarkCircledIcon,
  CheckCircledIcon,
} from "@radix-ui/react-icons";
import {
  Box,
  Flex,
  Text,
  Button,
  TextArea,
  RadioGroup,
  Checkbox,
} from "@radix-ui/themes";
import { ToolCard, ToolStatus } from "./ToolCard";
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
          <Text weight="medium" size="2" mb="2" as="p">
            {question.text}
          </Text>
          <RadioGroup.Root
            value={typeof value === "string" ? value : ""}
            onValueChange={onChange}
          >
            <Flex gap="3">
              <RadioGroup.Item value="Yes">Yes</RadioGroup.Item>
              <RadioGroup.Item value="No">No</RadioGroup.Item>
            </Flex>
          </RadioGroup.Root>
        </Box>
      );

    case "single_select":
      return (
        <Box className={styles.questionItem}>
          <Text weight="medium" size="2" mb="2" as="p">
            {question.text}
          </Text>
          <RadioGroup.Root
            value={typeof value === "string" ? value : ""}
            onValueChange={onChange}
          >
            <Flex direction="column" gap="2">
              {question.options?.map((opt) => (
                <RadioGroup.Item key={opt} value={opt}>
                  {opt}
                </RadioGroup.Item>
              ))}
            </Flex>
          </RadioGroup.Root>
        </Box>
      );

    case "multi_select":
      return (
        <Box className={styles.questionItem}>
          <Text weight="medium" size="2" mb="2" as="p">
            {question.text}
          </Text>
          <Flex direction="column" gap="2">
            {question.options?.map((opt) => (
              <Flex key={opt} align="center" gap="2">
                <Checkbox
                  checked={Array.isArray(value) && value.includes(opt)}
                  onCheckedChange={(checked) => {
                    const current = Array.isArray(value) ? value : [];
                    if (checked === true) {
                      onChange([...current, opt]);
                    } else {
                      onChange(current.filter((v) => v !== opt));
                    }
                  }}
                />
                <Text size="2">{opt}</Text>
              </Flex>
            ))}
          </Flex>
        </Box>
      );

    case "free_text":
      return (
        <Box className={styles.questionItem}>
          <Text weight="medium" size="2" mb="2" as="p">
            {question.text}
          </Text>
          <TextArea
            value={typeof value === "string" ? value : ""}
            onChange={(e) => onChange(e.target.value)}
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
  const [isOpen, setIsOpen] = useState(true);
  const [answers, setAnswers] = useState<Record<string, string | string[]>>({});
  const [additionalText, setAdditionalText] = useState("");
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

  const handleToggle = useCallback(() => {
    setIsOpen((prev) => !prev);
  }, []);

  useEffect(() => {
    if (nextUserMessage && !answeredViaForm && !hasCollapsedManualRef.current) {
      hasCollapsedManualRef.current = true;
      setIsOpen(false);
    }
  }, [nextUserMessage, answeredViaForm]);

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

  const hasNextMessage = !!nextUserMessage;

  if (!hasNextMessage && data) {
    return (
      <ToolCard
        icon={<QuestionMarkCircledIcon />}
        summary="Questions for you"
        status={status}
        isOpen={isOpen}
        onToggle={handleToggle}
      >
        <Box className={styles.content}>
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
              <TextArea
                value={additionalText}
                onChange={(e) => setAdditionalText(e.target.value)}
                placeholder="Add any extra context..."
              />
            </Box>

            <Button onClick={handleSubmit} size="2">
              Submit Answers
            </Button>
          </Flex>
        </Box>
      </ToolCard>
    );
  }

  if (answeredViaForm && data && parsedAnswers) {
    return (
      <ToolCard
        icon={<CheckCircledIcon />}
        summary="Questions answered"
        status="success"
        isOpen={isOpen}
        onToggle={handleToggle}
      >
        <Box className={styles.content}>
          <Flex direction="column" gap="2">
            {data.questions.map((q) => (
              <Box key={q.id}>
                <Text weight="medium" size="2">
                  {q.text}
                </Text>
                <Text color="gray" size="2" ml="2">
                  → {parsedAnswers[q.id] || "(no answer)"}
                </Text>
              </Box>
            ))}
            {parsedAnswers.__additional__ && (
              <Box mt="2">
                <Text size="2" color="gray" style={{ fontStyle: "italic" }}>
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
      icon={<QuestionMarkCircledIcon />}
      summary="Questions (answered manually)"
      status="success"
      isOpen={isOpen}
      onToggle={handleToggle}
    >
      {data && (
        <Box className={styles.content}>
          <Flex direction="column" gap="1">
            {data.questions.map((q) => (
              <Text key={q.id} size="2">
                • {q.text}
              </Text>
            ))}
          </Flex>
        </Box>
      )}
    </ToolCard>
  );
};

export default AskQuestionsTool;
