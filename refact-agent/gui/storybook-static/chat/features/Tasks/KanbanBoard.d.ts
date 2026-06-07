import React from "react";
import type { TaskBoard, BoardCard } from "../../services/refact/tasks";
interface KanbanBoardProps {
    board: TaskBoard;
    onCardClick?: (card: BoardCard) => void;
    onAgentClick?: (card: BoardCard) => void;
}
export declare const KanbanBoard: React.FC<KanbanBoardProps>;
export {};
