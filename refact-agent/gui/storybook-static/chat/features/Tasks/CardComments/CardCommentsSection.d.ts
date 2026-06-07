import React from "react";
import { type CardComment } from "../../../services/refact/tasks";
interface CardCommentsSectionProps {
    taskId: string;
    cardId: string;
    comments: CardComment[];
}
export declare const CardCommentsSection: React.FC<CardCommentsSectionProps>;
export {};
