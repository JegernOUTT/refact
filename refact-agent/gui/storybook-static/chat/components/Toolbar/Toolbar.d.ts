import { JSX } from 'react/jsx-runtime';
export type DashboardTab = {
    type: "dashboard";
};
export type ChatTab = {
    type: "chat";
    id: string;
};
export type TaskTab = {
    type: "task";
    taskId: string;
    taskName: string;
};
export type Tab = DashboardTab | ChatTab | TaskTab;
export type ToolbarProps = {
    activeTab: Tab;
};
export declare const Toolbar: ({ activeTab }: ToolbarProps) => JSX.Element;
