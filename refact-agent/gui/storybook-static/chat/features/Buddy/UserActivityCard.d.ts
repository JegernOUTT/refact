import React from "react";
import { type UserActivityResponse } from "../../services/refact/buddy";
interface UserActivityCardProps {
    activity?: UserActivityResponse;
    hours?: number;
}
export declare const UserActivityCard: React.FC<UserActivityCardProps>;
export {};
