import { createSlice, PayloadAction } from "@reduxjs/toolkit";

export interface HistoryList {
  name: "history";
}

export interface ChatPage {
  name: "chat";
}

export interface FIMDebugPage {
  name: "fill in the middle debug page";
}

export interface DocumentationSettingsPage {
  name: "documentation settings";
}

export interface ChatThreadHistoryPage {
  name: "thread history page";
  // causes a bug with other pages
  chatId: string;
}

export interface LoginPage {
  name: "login page";
}

export interface ProvidersPage {
  name: "providers page";
}

export interface TasksListPage {
  name: "tasks list";
}

export interface TaskWorkspacePage {
  name: "task workspace";
  taskId: string;
}

export interface TaskAgentPage {
  name: "task agent";
  taskId: string;
  agentId: string;
  chatId: string;
}

export interface KnowledgeGraphPage {
  name: "knowledge graph";
}

export interface CustomizationPage {
  name: "customization";
  kind?: "modes" | "subagents" | "toolbox_commands" | "code_lens";
  configId?: string;
  draftId?: string;
}

export interface DefaultModelsPage {
  name: "default models";
  draftId?: string;
}

export interface StatsDashboardPage {
  name: "stats dashboard";
}

export interface ExtensionsPage {
  name: "extensions";
  tab?: "skills" | "commands" | "hooks";
  itemId?: string;
  draftId?: string;
}

export interface MCPMarketplacePage {
  name: "mcp marketplace";
}

export interface SkillsMarketplacePage {
  name: "skills marketplace";
}

export interface CommandsMarketplacePage {
  name: "commands marketplace";
}

export interface SubagentsMarketplacePage {
  name: "subagents marketplace";
}

export interface MarketplaceHubPage {
  name: "marketplace hub";
}

export interface BuddyPage {
  name: "buddy";
  draftId?: string;
}

export interface IntegrationsSetupPage {
  name: "integrations page";
  projectPath?: string;
  integrationName?: string;
  integrationPath?: string;
  shouldIntermediatePageShowUp?: boolean;
  wasOpenedThroughChat?: boolean;
}

export type Page =
  | ChatPage
  | HistoryList
  | FIMDebugPage
  | DocumentationSettingsPage
  | ChatThreadHistoryPage
  | IntegrationsSetupPage
  | ProvidersPage
  | LoginPage
  | TasksListPage
  | TaskWorkspacePage
  | TaskAgentPage
  | KnowledgeGraphPage
  | CustomizationPage
  | DefaultModelsPage
  | StatsDashboardPage
  | ExtensionsPage
  | MCPMarketplacePage
  | SkillsMarketplacePage
  | CommandsMarketplacePage
  | SubagentsMarketplacePage
  | MarketplaceHubPage
  | BuddyPage;

export function isIntegrationSetupPage(
  page: Page,
): page is IntegrationsSetupPage {
  return page.name === "integrations page";
}

export function isExtensionsPage(page: Page): page is ExtensionsPage {
  return page.name === "extensions";
}

export type PageSliceState = Page[];

const initialState: PageSliceState = [{ name: "login page" }];

export const pagesSlice = createSlice({
  name: "pages",
  initialState,
  reducers: {
    pop: (state) => {
      state.pop();
    },
    push: (state, action: PayloadAction<Page>) => {
      state.push(action.payload);
    },
    popBackTo: (state, action: PayloadAction<Page>) => {
      const pageIndex = state.findIndex((page) => {
        if (
          isIntegrationSetupPage(action.payload) &&
          isIntegrationSetupPage(page) &&
          action.payload.projectPath === page.projectPath &&
          action.payload.integrationName === page.integrationName
        ) {
          return true;
        } else if (isIntegrationSetupPage(action.payload)) {
          return false;
        }
        return page.name === action.payload.name;
      });
      if (pageIndex === -1) {
        state.push(action.payload);
        return;
      }
      state.length = pageIndex + 1;
    },

    change: (state, action: PayloadAction<Page>) => {
      state.pop();
      state.push(action.payload);
    },
  },
  selectors: {
    isPageInHistory: (state, name: string) => {
      return state.some((page) => page.name === name);
    },
    selectPages: (state) => state,

    selectCurrentPage: (state) => {
      if (state.length === 0) return undefined;
      return state[state.length - 1];
    },
  },
});

export const { pop, push, popBackTo, change } = pagesSlice.actions;
export const { selectPages, isPageInHistory, selectCurrentPage } =
  pagesSlice.selectors;
