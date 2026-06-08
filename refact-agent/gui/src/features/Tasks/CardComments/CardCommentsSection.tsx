import React, { useState } from "react";
import classNames from "classnames";
import {
  Box,
  Flex,
  Text,
  Badge,
  Button,
  TextArea,
  Tooltip,
  Spinner,
} from "@radix-ui/themes";
import { FileTextIcon, PersonIcon } from "@radix-ui/react-icons";
import { AgentStatusDot } from "../AgentStatusDot";
import { Markdown } from "../../../components/Markdown";
import {
  useAddCardCommentMutation,
  type CardComment,
} from "../../../services/refact/tasks";
import styles from "./CardCommentsSection.module.css";

function formatRelativeTime(timestamp: string): string {
  const diffMs = Date.now() - new Date(timestamp).getTime();
  const diffSeconds = Math.floor(diffMs / 1000);
  if (diffSeconds < 60) return "just now";
  const diffMinutes = Math.floor(diffSeconds / 60);
  if (diffMinutes < 60) return `${diffMinutes}m ago`;
  const diffHours = Math.floor(diffMinutes / 60);
  if (diffHours < 24) return `${diffHours}h ago`;
  return `${Math.floor(diffHours / 24)}d ago`;
}

function threadComments(comments: CardComment[]): CardComment[] {
  const topLevel = comments.filter((c) => c.reply_to === null);
  const replies = comments.filter((c) => c.reply_to !== null);
  const result: CardComment[] = [];
  for (const top of topLevel) {
    result.push(top);
    result.push(...replies.filter((r) => r.reply_to === top.id));
  }
  const handled = new Set(result.map((c) => c.id));
  for (const r of replies) {
    if (!handled.has(r.id)) result.push(r);
  }
  return result;
}

interface CommentItemProps {
  comment: CardComment;
  onReply: () => void;
  isReply: boolean;
}

const CommentItem: React.FC<CommentItemProps> = ({
  comment,
  onReply,
  isReply,
}) => {
  const relativeTime = formatRelativeTime(comment.timestamp);
  const fullTime = new Date(comment.timestamp).toLocaleString();
  const authorDisplay = comment.author_id
    ? comment.author_id.slice(0, 8)
    : "anonymous";

  const roleIcon =
    comment.author_role === "planner" ? (
      <Badge size="1" color="violet">
        <FileTextIcon />
      </Badge>
    ) : comment.author_role === "agents" ? (
      <AgentStatusDot status="doing" size="small" />
    ) : comment.author_role === "user" ? (
      <Badge size="1" color="green">
        <PersonIcon />
      </Badge>
    ) : (
      <Badge size="1" color="gray">
        sys
      </Badge>
    );

  return (
    <Box
      style={isReply ? { marginLeft: "var(--space-4)" } : undefined}
      className={classNames(styles.commentItem, "rf-enter-rise")}
    >
      <Flex align="center" gap="1" mb="1" wrap="wrap">
        {roleIcon}
        <Badge size="1" variant="soft">
          {comment.author_role}
        </Badge>
        <Text size="1" color="gray">
          {authorDisplay}
        </Text>
        <Tooltip content={fullTime}>
          <Text size="1" color="gray">
            {relativeTime}
          </Text>
        </Tooltip>
      </Flex>
      <Box className={styles.commentBody}>
        <Markdown canHaveInteractiveElements={false}>{comment.body}</Markdown>
      </Box>
      <Flex justify="end">
        <Button size="1" variant="ghost" onClick={onReply}>
          Reply
        </Button>
      </Flex>
    </Box>
  );
};

interface CardCommentsSectionProps {
  taskId: string;
  cardId: string;
  comments: CardComment[];
}

export const CardCommentsSection: React.FC<CardCommentsSectionProps> = ({
  taskId,
  cardId,
  comments,
}) => {
  const [body, setBody] = useState("");
  const [replyTo, setReplyTo] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [addCardComment, { isLoading: isSubmitting }] =
    useAddCardCommentMutation();

  const handleSubmit = async () => {
    setError(null);
    try {
      await addCardComment({
        taskId,
        cardId,
        body: body.trim(),
        authorRole: "user",
        replyTo: replyTo ?? undefined,
      }).unwrap();
      setBody("");
      setReplyTo(null);
    } catch (err) {
      const data =
        err &&
        typeof err === "object" &&
        "data" in err &&
        err.data &&
        typeof err.data === "object"
          ? (err.data as Record<string, unknown>)
          : null;
      const msg =
        data && typeof data.error === "string" ? data.error : "Unknown error";
      setError(`Failed to add comment: ${msg}`);
    }
  };

  const threaded = threadComments(comments);

  return (
    <Box>
      <Flex justify="between" align="center">
        <Text size="2" weight="medium" color="gray">
          Comments ({comments.length})
        </Text>
      </Flex>

      <Flex
        direction="column"
        gap="2"
        mt="2"
        className={classNames(styles.commentList, "rf-stagger")}
      >
        {threaded.length === 0 ? (
          <Text size="1" color="gray">
            No comments yet.
          </Text>
        ) : (
          threaded.map((comment) => (
            <CommentItem
              key={comment.id}
              comment={comment}
              onReply={() => setReplyTo(comment.id)}
              isReply={comment.reply_to !== null}
            />
          ))
        )}
      </Flex>

      <Box mt="3" className={styles.composer}>
        {replyTo && (
          <Flex align="center" gap="2" mb="1">
            <Badge size="1" variant="soft">
              Replying to {replyTo.slice(0, 8)}
            </Badge>
            <Button size="1" variant="ghost" onClick={() => setReplyTo(null)}>
              Cancel reply
            </Button>
          </Flex>
        )}
        <TextArea
          value={body}
          onChange={(e) => setBody(e.target.value)}
          placeholder="Add a comment..."
          disabled={isSubmitting}
        />
        <Flex justify="end" mt="1">
          <Button
            size="1"
            disabled={body.trim().length === 0 || isSubmitting}
            onClick={() => void handleSubmit()}
          >
            {isSubmitting ? <Spinner size="1" /> : "Comment"}
          </Button>
        </Flex>
        {error && (
          <Text size="1" color="red" mt="1">
            {error}
          </Text>
        )}
      </Box>
    </Box>
  );
};
