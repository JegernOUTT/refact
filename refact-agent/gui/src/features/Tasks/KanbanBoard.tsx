/* eslint-disable react/prop-types */
import React, { useCallback } from "react";
import { Flex, Box, Text, Card, Badge, Heading, Tooltip } from "@radix-ui/themes";
import type { TaskBoard, BoardCard, BoardColumn } from "../../services/refact/tasks";
import styles from "./Tasks.module.css";

const getPriorityColor = (priority: string): "red" | "orange" | "gray" => {
  if (priority === "P0") return "red";
  if (priority === "P1") return "orange";
  return "gray";
};

const columnColors: Record<string, string> = {
  planned: "var(--gray-5)",
  doing: "var(--blue-5)",
  done: "var(--green-5)",
  failed: "var(--red-5)",
};

interface KanbanCardProps {
  card: BoardCard;
  onClick?: (card: BoardCard) => void;
}

const KanbanCard: React.FC<KanbanCardProps> = ({ card, onClick }) => {
  const handleClick = useCallback(() => {
    onClick?.(card);
  }, [card, onClick]);

  const hasAgent = card.assignee !== null;
  const hasDeps = card.depends_on.length > 0;

  return (
    <Card
      className={styles.kanbanCard}
      onClick={handleClick}
      style={{ cursor: onClick ? "pointer" : "default" }}
    >
      <Flex direction="column" gap="1">
        <Flex justify="between" align="start">
          <Text size="2" weight="medium" style={{ flex: 1 }}>
            {card.title}
          </Text>
          <Badge color={getPriorityColor(card.priority)} size="1">
            {card.priority}
          </Badge>
        </Flex>

        <Flex gap="1" wrap="wrap">
          {hasAgent && (
            <Tooltip content={`Agent: ${card.assignee}`}>
              <Badge size="1" color="blue" variant="soft">
                🤖 Agent
              </Badge>
            </Tooltip>
          )}
          {hasDeps && (
            <Tooltip content={`Depends on: ${card.depends_on.join(", ")}`}>
              <Badge size="1" color="gray" variant="soft">
                ⛓️ {card.depends_on.length}
              </Badge>
            </Tooltip>
          )}
          {card.status_updates.length > 0 && (
            <Badge size="1" color="gray" variant="soft">
              📝 {card.status_updates.length}
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
}

const KanbanColumn: React.FC<KanbanColumnProps> = ({ column, cards, onCardClick }) => {
  return (
    <Flex
      direction="column"
      className={styles.kanbanColumn}
      style={{ borderTopColor: columnColors[column.id] || "var(--gray-5)" }}
    >
      <Flex justify="between" align="center" className={styles.kanbanColumnHeader}>
        <Heading size="1">{column.title}</Heading>
        <Badge size="1" color="gray">{cards.length}</Badge>
      </Flex>
      <Box className={styles.kanbanColumnContent}>
        <Flex direction="column" gap="1">
          {cards.map((card) => (
            <KanbanCard
              key={card.id}
              card={card}
              onClick={onCardClick}
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
}

export const KanbanBoard: React.FC<KanbanBoardProps> = ({ board, onCardClick }) => {
  const getCardsForColumn = useCallback((columnId: string): BoardCard[] => {
    return board.cards.filter(card => card.column === columnId);
  }, [board.cards]);

  return (
    <Flex className={styles.kanbanBoard}>
      {board.columns.map((column) => (
        <KanbanColumn
          key={column.id}
          column={column}
          cards={getCardsForColumn(column.id)}
          onCardClick={onCardClick}
        />
      ))}
    </Flex>
  );
};
