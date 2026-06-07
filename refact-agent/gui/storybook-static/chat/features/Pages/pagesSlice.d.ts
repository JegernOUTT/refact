import { Selector } from 'reselect';
import { WritableDraft } from 'immer';
import { Slice, ActionCreatorWithoutPayload, ActionCreatorWithPayload, ActionCreatorWithOptionalPayload, PayloadAction } from '@reduxjs/toolkit';
export interface HistoryList {
    name: "history";
}
export interface ChatPage {
    name: "chat";
}
export interface DocumentationSettingsPage {
    name: "documentation settings";
}
export interface ChatThreadHistoryPage {
    name: "thread history page";
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
export interface SchedulerPage {
    name: "scheduler";
    taskId?: string;
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
export type Page = ChatPage | HistoryList | DocumentationSettingsPage | ChatThreadHistoryPage | IntegrationsSetupPage | ProvidersPage | LoginPage | TasksListPage | TaskWorkspacePage | TaskAgentPage | SchedulerPage | KnowledgeGraphPage | CustomizationPage | DefaultModelsPage | StatsDashboardPage | ExtensionsPage | MCPMarketplacePage | SkillsMarketplacePage | CommandsMarketplacePage | SubagentsMarketplacePage | MarketplaceHubPage | BuddyPage;
export declare function isIntegrationSetupPage(page: Page): page is IntegrationsSetupPage;
export declare function isExtensionsPage(page: Page): page is ExtensionsPage;
export type PageSliceState = Page[];
export declare const pagesSlice: Slice<PageSliceState, {
    pop: (state: (WritableDraft<ChatPage> | WritableDraft<HistoryList> | WritableDraft<DocumentationSettingsPage> | WritableDraft<ChatThreadHistoryPage> | WritableDraft<IntegrationsSetupPage> | WritableDraft<ProvidersPage> | WritableDraft<LoginPage> | WritableDraft<TasksListPage> | WritableDraft<TaskWorkspacePage> | WritableDraft<TaskAgentPage> | WritableDraft<SchedulerPage> | WritableDraft<KnowledgeGraphPage> | WritableDraft<CustomizationPage> | WritableDraft<DefaultModelsPage> | WritableDraft<StatsDashboardPage> | WritableDraft<ExtensionsPage> | WritableDraft<MCPMarketplacePage> | WritableDraft<SkillsMarketplacePage> | WritableDraft<CommandsMarketplacePage> | WritableDraft<SubagentsMarketplacePage> | WritableDraft<MarketplaceHubPage> | WritableDraft<BuddyPage>)[]) => void;
    push: (state: (WritableDraft<ChatPage> | WritableDraft<HistoryList> | WritableDraft<DocumentationSettingsPage> | WritableDraft<ChatThreadHistoryPage> | WritableDraft<IntegrationsSetupPage> | WritableDraft<ProvidersPage> | WritableDraft<LoginPage> | WritableDraft<TasksListPage> | WritableDraft<TaskWorkspacePage> | WritableDraft<TaskAgentPage> | WritableDraft<SchedulerPage> | WritableDraft<KnowledgeGraphPage> | WritableDraft<CustomizationPage> | WritableDraft<DefaultModelsPage> | WritableDraft<StatsDashboardPage> | WritableDraft<ExtensionsPage> | WritableDraft<MCPMarketplacePage> | WritableDraft<SkillsMarketplacePage> | WritableDraft<CommandsMarketplacePage> | WritableDraft<SubagentsMarketplacePage> | WritableDraft<MarketplaceHubPage> | WritableDraft<BuddyPage>)[], action: PayloadAction<Page>) => void;
    popBackTo: (state: (WritableDraft<ChatPage> | WritableDraft<HistoryList> | WritableDraft<DocumentationSettingsPage> | WritableDraft<ChatThreadHistoryPage> | WritableDraft<IntegrationsSetupPage> | WritableDraft<ProvidersPage> | WritableDraft<LoginPage> | WritableDraft<TasksListPage> | WritableDraft<TaskWorkspacePage> | WritableDraft<TaskAgentPage> | WritableDraft<SchedulerPage> | WritableDraft<KnowledgeGraphPage> | WritableDraft<CustomizationPage> | WritableDraft<DefaultModelsPage> | WritableDraft<StatsDashboardPage> | WritableDraft<ExtensionsPage> | WritableDraft<MCPMarketplacePage> | WritableDraft<SkillsMarketplacePage> | WritableDraft<CommandsMarketplacePage> | WritableDraft<SubagentsMarketplacePage> | WritableDraft<MarketplaceHubPage> | WritableDraft<BuddyPage>)[], action: PayloadAction<Page>) => void;
    change: (state: (WritableDraft<ChatPage> | WritableDraft<HistoryList> | WritableDraft<DocumentationSettingsPage> | WritableDraft<ChatThreadHistoryPage> | WritableDraft<IntegrationsSetupPage> | WritableDraft<ProvidersPage> | WritableDraft<LoginPage> | WritableDraft<TasksListPage> | WritableDraft<TaskWorkspacePage> | WritableDraft<TaskAgentPage> | WritableDraft<SchedulerPage> | WritableDraft<KnowledgeGraphPage> | WritableDraft<CustomizationPage> | WritableDraft<DefaultModelsPage> | WritableDraft<StatsDashboardPage> | WritableDraft<ExtensionsPage> | WritableDraft<MCPMarketplacePage> | WritableDraft<SkillsMarketplacePage> | WritableDraft<CommandsMarketplacePage> | WritableDraft<SubagentsMarketplacePage> | WritableDraft<MarketplaceHubPage> | WritableDraft<BuddyPage>)[], action: PayloadAction<Page>) => void;
    openScheduler: (state: (WritableDraft<ChatPage> | WritableDraft<HistoryList> | WritableDraft<DocumentationSettingsPage> | WritableDraft<ChatThreadHistoryPage> | WritableDraft<IntegrationsSetupPage> | WritableDraft<ProvidersPage> | WritableDraft<LoginPage> | WritableDraft<TasksListPage> | WritableDraft<TaskWorkspacePage> | WritableDraft<TaskAgentPage> | WritableDraft<SchedulerPage> | WritableDraft<KnowledgeGraphPage> | WritableDraft<CustomizationPage> | WritableDraft<DefaultModelsPage> | WritableDraft<StatsDashboardPage> | WritableDraft<ExtensionsPage> | WritableDraft<MCPMarketplacePage> | WritableDraft<SkillsMarketplacePage> | WritableDraft<CommandsMarketplacePage> | WritableDraft<SubagentsMarketplacePage> | WritableDraft<MarketplaceHubPage> | WritableDraft<BuddyPage>)[], action: PayloadAction<{
        taskId?: string;
    } | undefined>) => void;
}, "pages", "pages", {
    isPageInHistory: (state: PageSliceState, name: string) => boolean;
    selectPages: (state: PageSliceState) => PageSliceState;
    selectCurrentPage: (state: PageSliceState) => Page | undefined;
}>;
export declare const pop: ActionCreatorWithoutPayload<"pages/pop">, push: ActionCreatorWithPayload<Page, "pages/push">, popBackTo: ActionCreatorWithPayload<Page, "pages/popBackTo">, change: ActionCreatorWithPayload<Page, "pages/change">, openScheduler: ActionCreatorWithOptionalPayload<{
    taskId?: string;
} | undefined, "pages/openScheduler">;
export declare const selectPages: Selector<{
    pages: PageSliceState;
}, PageSliceState, []> & {
    unwrapped: (state: PageSliceState) => PageSliceState;
}, isPageInHistory: Selector<{
    pages: PageSliceState;
}, boolean, [name: string]> & {
    unwrapped: (state: PageSliceState, name: string) => boolean;
}, selectCurrentPage: Selector<{
    pages: PageSliceState;
}, Page | undefined, []> & {
    unwrapped: (state: PageSliceState) => Page | undefined;
};
