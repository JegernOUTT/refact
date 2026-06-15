import type { LucideIcon } from "lucide-react";
import {
  Activity,
  Bird,
  BookOpen,
  Bot,
  Boxes,
  Brain,
  Bug,
  CircleAlert,
  Cog,
  Egg,
  Eye,
  FlaskConical,
  Flame,
  Ghost,
  GitBranch,
  Globe,
  ListTodo,
  MessageSquare,
  RefreshCw,
  Search,
  Sparkles,
  SquarePen,
  Workflow,
  Wrench,
  Zap,
} from "lucide-react";
import type { BuddyConversationEntry } from "./types";

/** Stage icon per STAGES index: Egg, Hatch, Sprite, Imp, Daemon, Sage, Archon. */
const STAGE_ICONS: LucideIcon[] = [
  Egg,
  Bird,
  Ghost,
  Zap,
  Flame,
  Brain,
  Sparkles,
];

export function stageIcon(stageIndex: number): LucideIcon {
  return STAGE_ICONS[stageIndex] ?? Sparkles;
}

export function conversationIcon(
  kind: BuddyConversationEntry["kind"],
): LucideIcon {
  switch (kind) {
    case "setup":
      return Wrench;
    case "workflow":
      return Workflow;
    case "system":
      return Cog;
    default:
      return MessageSquare;
  }
}

export function activityIcon(entry: {
  activity_type: string;
  failure_category?: string | null;
}): LucideIcon {
  if (entry.failure_category) return CircleAlert;
  if (entry.activity_type.startsWith("buddy_")) return Sparkles;
  if (entry.activity_type.startsWith("refact_")) return Bot;
  return Activity;
}

const SKILL_ICONS: Record<string, LucideIcon> = {
  edit: SquarePen,
  search: Search,
  debug: Bug,
  knowledge: BookOpen,
  browser: Globe,
  refactor: RefreshCw,
  test: FlaskConical,
  review: Eye,
  git: GitBranch,
  arch: Boxes,
  plan: ListTodo,
  mem: Brain,
};

export function skillIcon(skillId: string): LucideIcon {
  return SKILL_ICONS[skillId] ?? Sparkles;
}
