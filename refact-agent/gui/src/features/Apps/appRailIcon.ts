import {
  Activity,
  AppWindow,
  BarChart3,
  BookOpen,
  Bug,
  Calendar,
  CheckSquare,
  Database,
  FileText,
  GitBranch,
  Globe,
  Kanban,
  LayoutDashboard,
  ListTodo,
  Map as MapIcon,
  MessageSquare,
  Network,
  Notebook,
  PieChart,
  Server,
  Settings,
  Table,
  Terminal,
  Timer,
  Workflow,
  type LucideIcon,
} from "lucide-react";

const APP_ICONS: Record<string, LucideIcon | undefined> = {
  activity: Activity,
  "app-window": AppWindow,
  "bar-chart": BarChart3,
  "book-open": BookOpen,
  bug: Bug,
  calendar: Calendar,
  "check-square": CheckSquare,
  database: Database,
  "file-text": FileText,
  "git-branch": GitBranch,
  globe: Globe,
  kanban: Kanban,
  "layout-dashboard": LayoutDashboard,
  "list-todo": ListTodo,
  map: MapIcon,
  "message-square": MessageSquare,
  network: Network,
  notebook: Notebook,
  "pie-chart": PieChart,
  server: Server,
  settings: Settings,
  table: Table,
  terminal: Terminal,
  timer: Timer,
  workflow: Workflow,
};

export function appRailIcon(icon: string | null | undefined): LucideIcon {
  const match = /^lucide:([a-z0-9]+(?:-[a-z0-9]+)*)$/.exec(icon ?? "");
  if (!match || !Object.prototype.hasOwnProperty.call(APP_ICONS, match[1]))
    return AppWindow;
  return APP_ICONS[match[1]] ?? AppWindow;
}
