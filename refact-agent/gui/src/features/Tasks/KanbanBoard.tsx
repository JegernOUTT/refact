import React, { useCallback } from "react";
import {
  Flex,
  Box,
  Text,
  Card,
  Badge,
  Heading,
  Tooltip,
} from "@radix-ui/themes";
import type {
  TaskBoard,
  BoardCard,
  BoardColumn,
} from "../../services/refact/tasks";
import { FileTextIcon, Link2Icon, PersonIcon } from "@radix-ui/react-icons";
import { BranchIcon } from "../Worktrees/BranchIcon";
import styles from "./Tasks.module.css";

const getPriorityColor = (priority: string): "red" | "orange" | "gray" => {
  if (priority === "P0") return "red";
  if (priority === "P1") return "orange";
  return "gray";
};

function compactWorktreeLabel(label: string): string {
  const normalized = label.replace(/[\\/]+$/, "");
  const parts = normalized.split(/[\\/]/).filter(Boolean);
  if (parts.length <= 2) return normalized || label;
  return parts.slice(-2).join("/");
}

function cardWorktreeLabel(card: BoardCard): string | null {
  const label =
    card.agent_worktree_name ?? card.agent_branch ?? card.agent_worktree;
  return label ? compactWorktreeLabel(label) : null;
}

const columnColors: Record<string, string> = {
  planned: "var(--gray-5)",
  doing: "var(--blue-5)",
  done: "var(--green-5)",
  failed: "var(--red-5)",
};

interface KanbanCardProps {
  card: BoardCard;
  onClick?: (card: BoardCard) => void;
  onAgentClick?: (card: BoardCard) => void;
}

const KanbanCard: React.FC<KanbanCardProps> = ({
  card,
  onClick,
  onAgentClick,
}) => {
  const handleClick = useCallback(() => {
    onClick?.(card);
  }, [card, onClick]);

  const handleAgentClick = useCallback(
    (event: React.MouseEvent<HTMLButtonElement>) => {
      if (!card.agent_chat_id) return;
      event.preventDefault();
      event.stopPropagation();
      onAgentClick?.(card);
    },
    [card, onAgentClick],
  );

  const hasAgent = card.assignee !== null;
  const hasDeps = card.depends_on.length > 0;
  const worktree = cardWorktreeLabel(card);

  return (
    <Card
      className={styles.kanbanCard}
      onClick={handleClick}
      style={{ cursor: onClick ? "pointer" : "default" }}
    >
      <Flex direction="column" gap="2">
        <Flex
          justify="between"
          align="center"
          className={styles.kanbanCardTopRow}
        >
          <Badge size="1" color="gray" variant="soft">
            {card.id}
          </Badge>
          <Badge color={getPriorityColor(card.priority)} size="1">
            {card.priority}
          </Badge>
        </Flex>

        <Text size="2" weight="medium" className={styles.kanbanCardTitle}>
          {card.title}
        </Text>

        <Flex gap="1" wrap="wrap" className={styles.kanbanCardBadges}>
          {hasAgent && (
            <Tooltip content={`Agent: ${card.assignee}`}>
              <Badge
                size="1"
                color="blue"
                variant="soft"
                asChild={Boolean(card.agent_chat_id)}
              >
                {card.agent_chat_id ? (
                  <button
                    type="button"
                    className={styles.agentBadgeButton}
                    onClick={handleAgentClick}
                  >
                    <PersonIcon /> Agent
                  </button>
                ) : (
                  <span>
                    <PersonIcon /> Agent
                  </span>
                )}
              </Badge>
            </Tooltip>
          )}
          {worktree && (
            <Tooltip content={`Worktree: ${worktree}`}>
              <Badge size="1" color="green" variant="soft">
                <BranchIcon /> {worktree}
              </Badge>
            </Tooltip>
          )}
          {hasDeps && (
            <Tooltip content={`Depends on: ${card.depends_on.join(", ")}`}>
              <Badge size="1" color="gray" variant="soft">
                <Link2Icon /> {card.depends_on.length}
              </Badge>
            </Tooltip>
          )}
          {card.status_updates.length > 0 && (
            <Badge size="1" color="gray" variant="soft">
              <FileTextIcon /> {card.status_updates.length}
            </Badge>
          )}
        </Flex>
      </Flex>
    </Card>
  );
};

interface KanbanColumnProps {
  column: BoardColumn;
  cards: BoardCard[];
  onCardClick?: (card: BoardCard) => void;
  onAgentClick?: (card: BoardCard) => void;
}

const KanbanColumn: React.FC<KanbanColumnProps> = ({
  column,
  cards,
  onCardClick,
  onAgentClick,
}) => {
  return (
    <Flex
      direction="column"
      className={styles.kanbanColumn}
      style={{ borderTopColor: columnColors[column.id] || "var(--gray-5)" }}
    >
      <Flex
        justify="between"
        align="center"
        className={styles.kanbanColumnHeader}
      >
        <Heading size="1">{column.title}</Heading>
        <Badge size="1" color="gray">
          {cards.length}
        </Badge>
      </Flex>
      <Box className={styles.kanbanColumnContent}>
        <Flex direction="column" gap="1">
          {cards.map((card) => (
            <KanbanCard
              key={card.id}
              card={card}
              onClick={onCardClick}
              onAgentClick={onAgentClick}
            />
          ))}
        </Flex>
      </Box>
    </Flex>
  );
};

interface KanbanBoardProps {
  board: TaskBoard;
  onCardClick?: (card: BoardCard) => void;
  onAgentClick?: (card: BoardCard) => void;
}

export const KanbanBoard: React.FC<KanbanBoardProps> = ({
  board,
  onCardClick,
  onAgentClick,
}) => {
  const getCardsForColumn = useCallback(
    (columnId: string): BoardCard[] => {
      return board.cards.filter((card) => card.column === columnId);
    },
    [board.cards],
  );

  return (
    <Flex className={styles.kanbanBoard}>
      {board.columns.map((column) => (
        <KanbanColumn
          key={column.id}
          column={column}
          cards={getCardsForColumn(column.id)}
          onCardClick={onCardClick}
          onAgentClick={onAgentClick}
        />
      ))}
    </Flex>
  );
};
