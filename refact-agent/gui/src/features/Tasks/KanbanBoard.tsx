import React, { useCallback, useId, useState } from "react";
import classNames from "classnames";
import { FileText, Link2, MoreHorizontal, Plus, User } from "lucide-react";
import {
  Badge,
  Button,
  Card,
  Field,
  FieldSelect,
  FieldText,
  Icon,
  IconButton,
  Menu,
} from "../../components/ui";
import type {
  TaskBoard,
  BoardCard,
  BoardColumn,
} from "../../services/refact/tasks";
import { BranchIcon } from "../Worktrees/BranchIcon";
import { CardOutcomeBadge } from "./CardOutcomeBadge";
import { CardAttentionBadge } from "./CardAttentionBadge";
import styles from "./Tasks.module.css";
import managementStyles from "./CardManagement.module.css";
import type { CardEditTarget } from "./cardForm";

type BadgeTone = React.ComponentProps<typeof Badge>["tone"];

const priorityTone = (priority: string): BadgeTone => {
  if (priority === "P0") return "danger";
  if (priority === "P1") return "warning";
  return "muted";
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

function columnToneClass(columnId: string): string {
  if (columnId === "doing") return styles.kanbanColumnDoing;
  if (columnId === "done") return styles.kanbanColumnDone;
  if (columnId === "failed") return styles.kanbanColumnFailed;
  return styles.kanbanColumnPlanned;
}

interface KanbanCardProps {
  card: BoardCard;
  cardOutcomes?: TaskBoard["card_outcomes"];
  onClick?: (card: BoardCard) => void;
  onAgentClick?: (card: BoardCard) => void;
  onManageCard?: (target: CardEditTarget) => void;
}

const KanbanCard: React.FC<KanbanCardProps> = ({
  card,
  cardOutcomes,
  onClick,
  onAgentClick,
  onManageCard,
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
    <Card animated="rise" className={styles.kanbanCard}>
      <div className={styles.kanbanCardFrame}>
        <div className={styles.kanbanCardTopRow}>
          <Badge tone="muted">{card.id}</Badge>
          <Badge tone={priorityTone(card.priority)}>{card.priority}</Badge>
          {onManageCard && (
            <Menu>
              <Menu.Trigger asChild>
                <IconButton
                  size="sm"
                  icon={MoreHorizontal}
                  aria-label={`Manage card ${card.id}`}
                />
              </Menu.Trigger>
              <Menu.Content align="end">
                <Menu.Item
                  onSelect={() => onManageCard({ mode: "edit", card })}
                >
                  Edit card
                </Menu.Item>
                <Menu.Item
                  onSelect={() => onManageCard({ mode: "move", card })}
                >
                  Move card
                </Menu.Item>
                <Menu.Separator />
                <Menu.Item
                  onSelect={() => onManageCard({ mode: "delete", card })}
                >
                  Delete card
                </Menu.Item>
              </Menu.Content>
            </Menu>
          )}
        </div>

        {onClick ? (
          <button
            type="button"
            className={classNames(
              styles.kanbanCardTitle,
              managementStyles.cardTitleButton,
            )}
            aria-label={`Open card ${card.id}: ${card.title}`}
            onClick={handleClick}
          >
            {card.title}
          </button>
        ) : (
          <span className={styles.kanbanCardTitle}>{card.title}</span>
        )}

        <div className={styles.kanbanCardBadges}>
          {hasAgent &&
            (card.agent_chat_id ? (
              <button
                type="button"
                className={styles.agentBadgeAction}
                title={`Agent: ${card.assignee}`}
                onClick={handleAgentClick}
              >
                <Icon icon={User} size="sm" tone="accent" /> Agent
              </button>
            ) : (
              <Badge tone="accent" title={`Agent: ${card.assignee}`}>
                <Icon icon={User} size="sm" tone="accent" /> Agent
              </Badge>
            ))}
          {worktree && (
            <Badge tone="success" title={`Worktree: ${worktree}`}>
              <BranchIcon /> {worktree}
            </Badge>
          )}
          {hasDeps && (
            <Badge
              tone="muted"
              title={`Depends on: ${card.depends_on.join(", ")}`}
            >
              <Icon icon={Link2} size="sm" tone="muted" />
              {card.depends_on.length}
            </Badge>
          )}
          {card.status_updates.length > 0 && (
            <Badge tone="muted">
              <Icon icon={FileText} size="sm" tone="muted" />
              {card.status_updates.length}
            </Badge>
          )}
          <CardOutcomeBadge cardId={card.id} cardOutcomes={cardOutcomes} />
          <CardAttentionBadge card={card} />
        </div>
      </div>
    </Card>
  );
};

interface KanbanColumnProps {
  column: BoardColumn;
  cards: BoardCard[];
  totalCards: number;
  cardOutcomes?: TaskBoard["card_outcomes"];
  onCardClick?: (card: BoardCard) => void;
  onAgentClick?: (card: BoardCard) => void;
  onManageCard?: (target: CardEditTarget) => void;
}

const KanbanColumn: React.FC<KanbanColumnProps> = ({
  column,
  cards,
  totalCards,
  cardOutcomes,
  onCardClick,
  onAgentClick,
  onManageCard,
}) => {
  return (
    <section
      className={classNames(
        styles.kanbanColumn,
        columnToneClass(column.id),
        "rf-enter-rise",
      )}
    >
      <header className={styles.kanbanColumnHeader}>
        <h3 className={styles.kanbanColumnTitle}>{column.title}</h3>
        <Badge
          tone="muted"
          title={`${cards.length} shown of ${totalCards} loaded cards`}
        >
          {cards.length === totalCards
            ? totalCards
            : `${cards.length}/${totalCards}`}
        </Badge>
      </header>
      <div className={classNames(styles.kanbanColumnContent, "rf-stagger")}>
        {cards.map((card) => (
          <KanbanCard
            key={card.id}
            card={card}
            cardOutcomes={cardOutcomes}
            onClick={onCardClick}
            onAgentClick={onAgentClick}
            onManageCard={onManageCard}
          />
        ))}
      </div>
    </section>
  );
};

interface KanbanBoardProps {
  board: TaskBoard;
  onCardClick?: (card: BoardCard) => void;
  onAgentClick?: (card: BoardCard) => void;
  onManageCard?: (target: CardEditTarget) => void;
}

export const KanbanBoard: React.FC<KanbanBoardProps> = ({
  board,
  onCardClick,
  onAgentClick,
  onManageCard,
}) => {
  const fieldId = useId();
  const [filter, setFilter] = useState("");
  const [sort, setSort] = useState("board");
  const query = filter.trim().toLowerCase();
  const visibleCards = board.cards.filter((card) =>
    `${card.id} ${card.title} ${card.priority} ${
      card.instructions
    } ${card.target_files.join(" ")}`
      .toLowerCase()
      .includes(query),
  );
  if (sort === "priority")
    visibleCards.sort(
      (left, right) =>
        left.priority.localeCompare(right.priority) ||
        left.id.localeCompare(right.id),
    );
  if (sort === "title")
    visibleCards.sort((left, right) => left.title.localeCompare(right.title));
  const getCardsForColumn = useCallback(
    (columnId: string): BoardCard[] => {
      return board.cards.filter((card) => card.column === columnId);
    },
    [board.cards],
  );

  return (
    <>
      <div className={managementStyles.toolbar}>
        {onManageCard && (
          <Button
            size="sm"
            variant="soft"
            leftIcon={Plus}
            onClick={() => onManageCard({ mode: "create" })}
          >
            Create card
          </Button>
        )}
        <Field
          label="Filter cards"
          htmlFor={`${fieldId}-filter`}
          className={managementStyles.filter}
        >
          <FieldText
            id={`${fieldId}-filter`}
            value={filter}
            onChange={setFilter}
            placeholder="ID, title, priority, or path"
          />
        </Field>
        <Field
          label="Sort cards"
          htmlFor={`${fieldId}-sort`}
          className={managementStyles.sort}
        >
          <FieldSelect
            id={`${fieldId}-sort`}
            value={sort}
            onChange={setSort}
            options={[
              { value: "board", label: "Board order" },
              { value: "priority", label: "Priority" },
              { value: "title", label: "Title" },
            ]}
          />
        </Field>
        <div className={managementStyles.summary} role="status">
          Showing {visibleCards.length} of {board.cards.length} loaded cards.
          Counts describe this response, not hidden or unavailable cards.
        </div>
      </div>
      <div className={classNames(styles.kanbanBoard, "scrollX", "rf-enter")}>
        {board.columns.map((column) => (
          <KanbanColumn
            key={column.id}
            column={column}
            cards={visibleCards.filter((card) => card.column === column.id)}
            totalCards={getCardsForColumn(column.id).length}
            cardOutcomes={board.card_outcomes}
            onCardClick={onCardClick}
            onAgentClick={onAgentClick}
            onManageCard={onManageCard}
          />
        ))}
      </div>
    </>
  );
};
