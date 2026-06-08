import type { LucideIcon } from "lucide-react";
import {
  BookOpen,
  Bot,
  Cable,
  Paintbrush,
  Plug,
  Settings,
  Sparkles,
  Timer,
} from "lucide-react";

export type SettingsSectionId =
  | "general"
  | "providers"
  | "models"
  | "customization"
  | "integrations"
  | "scheduler"
  | "documentation"
  | "extensions";

export interface SettingsSectionDef {
  id: SettingsSectionId;
  label: string;
  icon: LucideIcon;
}

export const SETTINGS_SECTIONS: SettingsSectionDef[] = [
  { id: "general", label: "General", icon: Settings },
  { id: "providers", label: "Providers", icon: Plug },
  { id: "models", label: "Models", icon: Bot },
  { id: "customization", label: "Customization", icon: Paintbrush },
  { id: "integrations", label: "Integrations", icon: Cable },
  { id: "scheduler", label: "Scheduler", icon: Timer },
  { id: "documentation", label: "Documentation", icon: BookOpen },
  { id: "extensions", label: "Extensions", icon: Sparkles },
];
