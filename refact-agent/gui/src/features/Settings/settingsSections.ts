import type { LucideIcon } from "lucide-react";
import {
  Bot,
  Cable,
  Chrome,
  Gauge,
  Paintbrush,
  Plug,
  ScanSearch,
  Settings,
  ShieldCheck,
  Sparkles,
  Store,
  Terminal,
  Timer,
} from "lucide-react";

export type SettingsSectionId =
  | "general"
  | "privacy"
  | "shell"
  | "indexing"
  | "browser"
  | "limits"
  | "providers"
  | "models"
  | "customization"
  | "integrations"
  | "scheduler"
  | "extensions"
  | "marketplace";

export interface SettingsSectionDef {
  id: SettingsSectionId;
  label: string;
  icon: LucideIcon;
}

export const SETTINGS_SECTIONS: SettingsSectionDef[] = [
  { id: "general", label: "General", icon: Settings },
  { id: "privacy", label: "Privacy & Access", icon: ShieldCheck },
  { id: "shell", label: "Shell", icon: Terminal },
  { id: "indexing", label: "Indexing", icon: ScanSearch },
  { id: "browser", label: "Browser", icon: Chrome },
  { id: "limits", label: "Limits & Budgets", icon: Gauge },
  { id: "providers", label: "Providers", icon: Plug },
  { id: "models", label: "Models", icon: Bot },
  { id: "customization", label: "Customization", icon: Paintbrush },
  { id: "integrations", label: "Integrations", icon: Cable },
  { id: "scheduler", label: "Scheduler", icon: Timer },
  { id: "extensions", label: "Extensions", icon: Sparkles },
  { id: "marketplace", label: "Marketplace", icon: Store },
];
