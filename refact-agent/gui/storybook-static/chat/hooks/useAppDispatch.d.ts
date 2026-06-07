import { BuddyConversationCreateRequest, BuddyErrorReport, BuddyOpportunityDismissResponse, CreateDraftRequest, UserActionPayload, UserActivityResponse, FrontendErrorReport, MemoryOpsState } from '../services/refact/buddy';
import { BuddySnapshot, BuddySettings, BuddyCareRequest, BuddyCareResponse, BuddyQuestAcceptResponse, BuddyPersonalityRerollResponse, BuddyActivityEntry, BuddyConversationEntry, BuddyConversationMeta, OpportunityStatus, BuddyOpportunity, BuddyOpportunityAcceptResponse, BuddyPulse, BuddyDraft } from '../features/Buddy/types';
import { PreviewResult } from '../services/refact/memoryEnrichment';
import { MarketplacesResponse, PluginListResponse, InstalledPluginsResponse } from '../services/refact/plugins';
import { ExtRegistryResponse, SkillDetail, CommandDetail, HooksDetail } from '../services/refact/extensions';
import { ProjectInformationConfig, ProjectInformationPreviewResponse } from '../services/refact/projectInformation';
import { RegistryResponse, ConfigKind, ConfigDetailResponse, SaveConfigResponse, DeleteConfigResponse } from '../services/refact/customization';
import { ChatModesResponse } from '../services/refact/chatModes';
import { MCPServerInfo } from '../services/refact/mcpServerInfo';
import { CronTask, CreateCronRequest, CreateCronResponse, DeleteCronRequest, DeleteCronResponse } from '../services/refact/schedulerApi';
import { PreviewCheckpointsPayload, PreviewCheckpointsResponse, RestoreCheckpointsPayload, RestoreCheckpointsResponse } from '../features/Checkpoints/types';
import { EngineApiConfig } from '../services/refact/apiUrl';
import { StatsSummary, StatsEventsParams, StatsEventsResponse } from '../features/StatsDashboard';
import { SidebarState } from '../features/Sidebar/sidebarSlice';
import { Config, FileInfo, CurrentProjectInfo, Snippet, Chat, CapsResponse, ToolEditResult } from '../events';
import { PersistPartial } from 'redux-persist/es/persistReducer';
import { TipOfTheDayState } from '../features/TipOfTheDay';
import { SchedulerState } from '../features/Scheduler/schedulerSlice';
import { NotificationsState } from '../features/Notifications';
import { BrowserState } from '../features/Browser';
import { ConnectionState } from '../features/Connection';
import { TasksUIState } from '../features/Tasks';
import { PatchMeta } from '../features/PatchesAndDiffsTracker/patchesAndDiffsTrackerSlice';
import { CheckpointsMeta } from '../features/Checkpoints/checkpointsSlice';
import { IntegrationCachedFormData } from '../features/Integrations';
import { IntegrationWithIconResponse, Integration, MCPOAuthStartResponse, MCPOAuthStatusResponse, SystemPrompts, ToolGroup, ToolGroupUpdate, ToolConfirmationRequest, ToolConfirmationResponse, CompletionArgs, CommandCompletionResponse, CommandPreviewRequest, CommandPreviewResponse, ChatContextFile, LinksApiRequest, LinksForChatResponse, CommitLinkPayload, CommitResponse, CompressTrajectoryPost, KnowledgeGraphResponse, SuccessResponse, ConfiguredProvidersResponse, ProviderDetailResponse, ProviderSchemaResponse, ProviderModelsResponse, AvailableModelsResponse, ProviderScopedQueryRequiredArg, OpenRouterModelEndpointsResponse, ProviderScopedQueryArg, OpenRouterAccountInfoResponse, OpenRouterHealthResponse, ClaudeCodeUsageResponse, OpenAICodexUsageResponse, AddCustomModelRequest, OAuthStartResponse, OAuthExchangeResponse, ProviderDefaults, ProviderDefaultsUpdateRequest, GetModelsArgs, ModelsResponse, GetModelArgs, Model, GetModelDefaultsArgs, CompletionModelFamiliesResponse, UpdateModelRequestBody, DeleteModelRequestBody, TrajectoryMeta, TrajectoriesListParams, PaginatedTrajectories, TrajectoryData, TransformOptions, TransformPreviewResponse, TransformApplyResponse, HandoffOptions, HandoffPreviewResponse, HandoffApplyResponse, ModeTransitionApplyResponse, TaskMeta, CreateTaskRequest, TaskBoard, ReadyCardsResult, TrajectoryInfo, TaskMemoriesQuery, TaskMemoriesResponse, TaskMemoryFacetsResponse, PinTaskMemoryRequest, PinTaskMemoryResponse, ArchiveTaskMemoryRequest, ArchiveTaskMemoryResponse, TriageTaskMemoriesRequest, TriageTaskMemoriesResponse, TaskDocumentListResponse, TaskDocumentDetail, CreateTaskDocumentRequest, UpdateTaskDocumentRequest, AppendTaskDocumentRequest, DeleteTaskDocumentRequest, DeleteTaskDocumentResponse, PinTaskDocumentRequest, TaskDocumentHistoryResponse, BrowserStartRequest, BrowserStartResponse, BrowserStopRequest, BrowserStopResponse, BrowserScreenshotRequest, BrowserScreenshotResponse, BrowserContextRequest, BrowserContextResponse, BrowserCurlRequest, BrowserCurlResponse, BrowserElementPickRequest, BrowserElementPickResponse, BrowserElementPickResultRequest, BrowserElementPickResultResponse, BrowserRecordAnimationRequest, BrowserRecordAnimationResponse, BrowserHandoffRequest, BrowserHandoffResponse, BrowserStatusRequest, BrowserStatusResponse, BrowserAnnotateStartRequest, BrowserAnnotateStartResponse, BrowserAnnotateResultRequest, BrowserAnnotateResultResponse, BrowserAnnotateClearRequest, BrowserAnnotateClearResponse, BrowserContextEstimateRequest, BrowserContextEstimateResponse, BrowserActionRequest, BrowserActionResponse, WorktreeListResponse, WorktreeInventory, WorktreeCleanupRequest, WorktreeCleanupPlan, WorktreeCleanupResult, CreateWorktreeRequest, CreateWorktreeResponse, GetWorktreeRequest, WorktreeRecordView, GetWorktreeDiffRequest, WorktreeDiffResponse, MergeWorktreeRequest, MergeWorktreeResponse, DeleteWorktreeRequest, DeleteWorktreeResponse, OpenWorktreeResponse, SkillsStatusResponse, SetupStatusResponse, MarketplaceQueryParams, MarketplaceResponse, InstallRequest, InstallResponse, InstalledResponse, AutoNameRequest, AutoNameResponse, MarketplaceSource, SaveSourceRequest, DeleteSourceRequest, ConfigureSourceRequest, ExtensionMarketplaceSource, ExtensionMarketplaceResponse, ExtensionMarketplaceInstallResponse } from '../services/refact';
import { CombinedState, QueryDefinition, BaseQueryFn, FetchArgs, FetchBaseQueryError, FetchBaseQueryMeta, MutationDefinition } from '@reduxjs/toolkit/query';
import { PageSliceState } from '../features/Pages/pagesSlice';
import { InformationSliceState } from '../features/Errors/informationSlice';
import { ErrorSliceState } from '../features/Errors/errorsSlice';
import { BuddySliceState, BuddySettingsResponse } from '../features/Buddy/buddySlice';
import { HistoryState } from '../features/History/historySlice';
import { ThunkDispatch } from 'redux-thunk';
import { UnsubscribeListener } from '@reduxjs/toolkit';
import { Action, UnknownAction, Dispatch } from 'redux';
import { UseDispatch } from 'react-redux';
export declare const useAppDispatch: UseDispatch<((action: Action<"listenerMiddleware/add">) => UnsubscribeListener) & ThunkDispatch<{
    history: HistoryState;
    buddy: BuddySliceState;
    error: ErrorSliceState;
    information: InformationSliceState;
    pages: PageSliceState;
    integrationsApi: CombinedState<{
        getAllIntegrations: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "INTEGRATIONS" | "INTEGRATION" | "MCP_OAUTH", IntegrationWithIconResponse, "integrationsApi">;
        getMCPLogsByPath: QueryDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "INTEGRATIONS" | "INTEGRATION" | "MCP_OAUTH", {
            logs: string[];
        }, "integrationsApi">;
        getIntegrationByPath: QueryDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "INTEGRATIONS" | "INTEGRATION" | "MCP_OAUTH", Integration, "integrationsApi">;
        saveIntegration: MutationDefinition<{
            filePath: string;
            values: Integration["integr_values"];
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "INTEGRATIONS" | "INTEGRATION" | "MCP_OAUTH", unknown, "integrationsApi">;
        deleteIntegration: QueryDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "INTEGRATIONS" | "INTEGRATION" | "MCP_OAUTH", unknown, "integrationsApi">;
        mcpOauthStart: MutationDefinition<{
            config_path: string;
            scopes?: string[];
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "INTEGRATIONS" | "INTEGRATION" | "MCP_OAUTH", MCPOAuthStartResponse, "integrationsApi">;
        mcpOauthExchange: MutationDefinition<{
            session_id: string;
            code: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "INTEGRATIONS" | "INTEGRATION" | "MCP_OAUTH", {
            success: boolean;
        }, "integrationsApi">;
        mcpOauthLogout: MutationDefinition<{
            config_path: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "INTEGRATIONS" | "INTEGRATION" | "MCP_OAUTH", {
            success: boolean;
        }, "integrationsApi">;
        mcpOauthCancel: MutationDefinition<{
            session_id: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "INTEGRATIONS" | "INTEGRATION" | "MCP_OAUTH", {
            cancelled: boolean;
        }, "integrationsApi">;
        mcpOauthStatus: QueryDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "INTEGRATIONS" | "INTEGRATION" | "MCP_OAUTH", MCPOAuthStatusResponse, "integrationsApi">;
    }, "INTEGRATIONS" | "INTEGRATION" | "MCP_OAUTH", "integrationsApi">;
    integrations: {
        cachedForms: IntegrationCachedFormData;
    };
    checkpoints: CheckpointsMeta;
    patchesAndDiffsTracker: {
        patches: PatchMeta[];
    };
    tasksUI: TasksUIState;
    connection: ConnectionState;
    browser: BrowserState;
    notifications: NotificationsState;
    scheduler: SchedulerState;
    tipOfTheDay: TipOfTheDayState & PersistPartial;
    config: Config;
    active_file: FileInfo;
    current_project: CurrentProjectInfo;
    sidebar: SidebarState;
    selected_snippet: Snippet;
    chat: Chat;
    statsApi: CombinedState<{
        getStatsSummary: QueryDefinition<{
            from?: string;
            to?: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, StatsSummary, "statsApi">;
        getStatsEvents: QueryDefinition<StatsEventsParams, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, StatsEventsResponse, "statsApi">;
    }, never, "statsApi">;
    caps: CombinedState<{
        getCaps: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, CapsResponse, "caps">;
    }, never, "caps">;
    prompts: CombinedState<{
        getPrompts: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, SystemPrompts, "prompts">;
    }, never, "prompts">;
    tools: CombinedState<{
        getToolGroups: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TOOL_GROUPS", ToolGroup[], "tools">;
        updateToolGroups: MutationDefinition<ToolGroupUpdate[], BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TOOL_GROUPS", {
            success: true;
        }, "tools">;
        checkForConfirmation: MutationDefinition<ToolConfirmationRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TOOL_GROUPS", ToolConfirmationResponse, "tools">;
        dryRunForEditTool: MutationDefinition<{
            toolName: string;
            toolArgs: Record<string, unknown>;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TOOL_GROUPS", ToolEditResult, "tools">;
    }, "TOOL_GROUPS", "tools">;
    commands: CombinedState<{
        getCommandCompletion: QueryDefinition<CompletionArgs, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, CommandCompletionResponse, "commands">;
        getCommandPreview: QueryDefinition<CommandPreviewRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, CommandPreviewResponse & {
            files: (ChatContextFile | string)[];
        }, "commands">;
    }, never, "commands">;
    pathApi: CombinedState<{
        getFullPath: QueryDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, string | null, "pathApi">;
        customizationPath: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, string, "pathApi">;
        privacyPath: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, string, "pathApi">;
        integrationsPath: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, string, "pathApi">;
    }, never, "pathApi">;
    pingApi: CombinedState<{
        ping: QueryDefinition<EngineApiConfig, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PING", string, "pingApi">;
        reset: MutationDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PING", null, "pingApi">;
    }, "PING", "pingApi">;
    linksApi: CombinedState<{
        getLinksForChat: QueryDefinition<LinksApiRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Chat_Links", LinksForChatResponse, "linksApi">;
        sendCommit: MutationDefinition<CommitLinkPayload, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Chat_Links", CommitResponse, "linksApi">;
    }, "Chat_Links", "linksApi">;
    checkpointsApi: CombinedState<{
        previewCheckpoints: MutationDefinition<PreviewCheckpointsPayload, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "CHECKPOINTS", PreviewCheckpointsResponse, "checkpointsApi">;
        restoreCheckpoints: MutationDefinition<RestoreCheckpointsPayload, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "CHECKPOINTS", RestoreCheckpointsResponse, "checkpointsApi">;
    }, "CHECKPOINTS", "checkpointsApi">;
    knowledgeApi: CombinedState<{
        compressMessages: MutationDefinition<CompressTrajectoryPost, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, {
            goal: string;
            trajectory: string;
        }, "knowledgeApi">;
    }, never, "knowledgeApi">;
    knowledgeGraphApi: CombinedState<{
        getKnowledgeGraph: QueryDefinition<{
            includeContent?: boolean;
        } | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "KnowledgeGraph" | "Memory", KnowledgeGraphResponse, "knowledgeGraphApi">;
        updateMemory: MutationDefinition<{
            file_path: string;
            title?: string;
            content: string;
            tags: string[];
            kind: string;
            filenames: string[];
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "KnowledgeGraph" | "Memory", SuccessResponse, "knowledgeGraphApi">;
        deleteMemory: MutationDefinition<{
            file_path: string;
            archive?: boolean;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "KnowledgeGraph" | "Memory", SuccessResponse, "knowledgeGraphApi">;
    }, "KnowledgeGraph" | "Memory", "knowledgeGraphApi">;
    providers: CombinedState<{
        getConfiguredProviders: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">;
        getProvider: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">;
        getProviderSchema: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">;
        getProviderModels: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">;
        getAvailableModels: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">;
        getOpenRouterModelEndpoints: QueryDefinition<ProviderScopedQueryRequiredArg & {
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">;
        getOpenRouterAccountInfo: QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">;
        getOpenRouterHealth: QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">;
        getClaudeCodeUsage: QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">;
        getOpenAICodexUsage: QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">;
        toggleModel: MutationDefinition<{
            providerName: string;
            modelId: string;
            enabled: boolean;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
            enabled: boolean;
        }, "providers">;
        setModelProvider: MutationDefinition<{
            providerName: string;
            modelId: string;
            selectedProvider?: string | null;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
            selected_provider?: string | null;
        }, "providers">;
        addCustomModel: MutationDefinition<{
            providerName: string;
            model: AddCustomModelRequest;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
        }, "providers">;
        removeCustomModel: MutationDefinition<{
            providerName: string;
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
        }, "providers">;
        updateProvider: MutationDefinition<{
            providerName: string;
            settings: Record<string, unknown>;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        oauthStart: MutationDefinition<{
            providerName: string;
            mode?: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthStartResponse, "providers">;
        oauthExchange: MutationDefinition<{
            providerName: string;
            session_id: string;
            code: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthExchangeResponse, "providers">;
        oauthLogout: MutationDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        deleteProvider: MutationDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        getDefaults: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">;
        updateDefaults: MutationDefinition<ProviderDefaultsUpdateRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
    }, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", "providers">;
    models: CombinedState<{
        getModels: QueryDefinition<GetModelsArgs, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MODELS" | "MODEL", ModelsResponse, "models">;
        getModel: QueryDefinition<GetModelArgs, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MODELS" | "MODEL", Model, "models">;
        getModelDefaults: QueryDefinition<GetModelDefaultsArgs, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MODELS" | "MODEL", Model, "models">;
        getCompletionModelFamilies: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MODELS" | "MODEL", CompletionModelFamiliesResponse, "models">;
        updateModel: MutationDefinition<UpdateModelRequestBody, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MODELS" | "MODEL", unknown, "models">;
        deleteModel: MutationDefinition<DeleteModelRequestBody, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MODELS" | "MODEL", unknown, "models">;
    }, "MODELS" | "MODEL", "models">;
    trajectoriesApi: CombinedState<{
        listTrajectoriesFirstPage: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Trajectory", TrajectoryMeta[], "trajectoriesApi">;
        listTrajectoriesPaginated: QueryDefinition<TrajectoriesListParams | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Trajectory", PaginatedTrajectories, "trajectoriesApi">;
        listAllTrajectories: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Trajectory", TrajectoryMeta[], "trajectoriesApi">;
        getTrajectory: QueryDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Trajectory", TrajectoryData, "trajectoriesApi">;
        getTrajectoryPath: QueryDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Trajectory", {
            path: string;
        }, "trajectoriesApi">;
        saveTrajectory: MutationDefinition<TrajectoryData, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Trajectory", undefined, "trajectoriesApi">;
        deleteTrajectory: MutationDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Trajectory", undefined, "trajectoriesApi">;
    }, "Trajectory", "trajectoriesApi">;
    trajectoryApi: CombinedState<{
        previewTransform: MutationDefinition<{
            chatId: string;
            options: TransformOptions;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, TransformPreviewResponse, "trajectoryApi">;
        applyTransform: MutationDefinition<{
            chatId: string;
            options: TransformOptions;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, TransformApplyResponse, "trajectoryApi">;
        previewHandoff: MutationDefinition<{
            chatId: string;
            options: HandoffOptions;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, HandoffPreviewResponse, "trajectoryApi">;
        applyHandoff: MutationDefinition<{
            chatId: string;
            options: HandoffOptions;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, HandoffApplyResponse, "trajectoryApi">;
        applyModeTransition: MutationDefinition<{
            chatId: string;
            targetMode: string;
            targetModeDescription?: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, ModeTransitionApplyResponse, "trajectoryApi">;
    }, never, "trajectoryApi">;
    tasksApi: CombinedState<{
        listTasks: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Tasks" | "Board" | "TaskTrajectories", TaskMeta[], "tasksApi">;
        createTask: MutationDefinition<CreateTaskRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Tasks" | "Board" | "TaskTrajectories", TaskMeta, "tasksApi">;
        getTask: QueryDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Tasks" | "Board" | "TaskTrajectories", TaskMeta, "tasksApi">;
        deleteTask: MutationDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Tasks" | "Board" | "TaskTrajectories", {
            deleted: boolean;
        }, "tasksApi">;
        updateTaskStatus: MutationDefinition<{
            taskId: string;
            status: TaskMeta["status"];
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Tasks" | "Board" | "TaskTrajectories", TaskMeta, "tasksApi">;
        getBoard: QueryDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Tasks" | "Board" | "TaskTrajectories", TaskBoard, "tasksApi">;
        patchBoard: MutationDefinition<{
            taskId: string;
            board: Partial<TaskBoard>;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Tasks" | "Board" | "TaskTrajectories", TaskBoard, "tasksApi">;
        getReadyCards: QueryDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Tasks" | "Board" | "TaskTrajectories", ReadyCardsResult, "tasksApi">;
        getOrchestratorInstructions: QueryDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Tasks" | "Board" | "TaskTrajectories", string, "tasksApi">;
        setOrchestratorInstructions: MutationDefinition<{
            taskId: string;
            content: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Tasks" | "Board" | "TaskTrajectories", {
            saved: boolean;
        }, "tasksApi">;
        listTaskTrajectories: QueryDefinition<{
            taskId: string;
            role: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Tasks" | "Board" | "TaskTrajectories", TrajectoryInfo[], "tasksApi">;
        createPlannerChat: MutationDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Tasks" | "Board" | "TaskTrajectories", {
            chat_id: string;
        }, "tasksApi">;
        deletePlannerChat: MutationDefinition<{
            taskId: string;
            chatId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Tasks" | "Board" | "TaskTrajectories", {
            deleted: boolean;
        }, "tasksApi">;
        createPlannerChatFromTransition: MutationDefinition<{
            taskId: string;
            sourceChatId: string;
            targetModeDescription?: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Tasks" | "Board" | "TaskTrajectories", {
            new_chat_id: string;
            messages_count: number;
        }, "tasksApi">;
        addCardComment: MutationDefinition<{
            taskId: string;
            cardId: string;
            body: string;
            authorRole: "user";
            replyTo?: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Tasks" | "Board" | "TaskTrajectories", TaskBoard, "tasksApi">;
        updateTaskMeta: MutationDefinition<{
            taskId: string;
            name?: string;
            baseBranch?: string;
            baseCommit?: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Tasks" | "Board" | "TaskTrajectories", TaskMeta, "tasksApi">;
    }, "Tasks" | "Board" | "TaskTrajectories", "tasksApi">;
    schedulerApi: CombinedState<{
        getCronTasks: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "CronTasks", CronTask[], "schedulerApi">;
        createCron: MutationDefinition<CreateCronRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "CronTasks", CreateCronResponse, "schedulerApi">;
        deleteCron: MutationDefinition<DeleteCronRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "CronTasks", DeleteCronResponse, "schedulerApi">;
    }, "CronTasks", "schedulerApi">;
    taskMemoriesApi: CombinedState<{
        listTaskMemories: QueryDefinition<TaskMemoriesQuery, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskMemories", TaskMemoriesResponse, "taskMemoriesApi">;
        getTaskMemoryFacets: QueryDefinition<{
            taskId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskMemories", TaskMemoryFacetsResponse, "taskMemoriesApi">;
        pinTaskMemory: MutationDefinition<PinTaskMemoryRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskMemories", PinTaskMemoryResponse, "taskMemoriesApi">;
        archiveTaskMemory: MutationDefinition<ArchiveTaskMemoryRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskMemories", ArchiveTaskMemoryResponse, "taskMemoriesApi">;
        triageTaskMemories: MutationDefinition<TriageTaskMemoriesRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskMemories", TriageTaskMemoriesResponse, "taskMemoriesApi">;
    }, "TaskMemories", "taskMemoriesApi">;
    taskDocumentsApi: CombinedState<{
        listTaskDocuments: QueryDefinition<{
            taskId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", TaskDocumentListResponse, "taskDocumentsApi">;
        getTaskDocument: QueryDefinition<{
            taskId: string;
            slug: string;
            version?: number;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", TaskDocumentDetail, "taskDocumentsApi">;
        createTaskDocument: MutationDefinition<CreateTaskDocumentRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", TaskDocumentDetail, "taskDocumentsApi">;
        updateTaskDocument: MutationDefinition<UpdateTaskDocumentRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", TaskDocumentDetail, "taskDocumentsApi">;
        appendTaskDocument: MutationDefinition<AppendTaskDocumentRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", TaskDocumentDetail, "taskDocumentsApi">;
        deleteTaskDocument: MutationDefinition<DeleteTaskDocumentRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", DeleteTaskDocumentResponse, "taskDocumentsApi">;
        pinTaskDocument: MutationDefinition<PinTaskDocumentRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", TaskDocumentDetail, "taskDocumentsApi">;
        getTaskDocumentHistory: QueryDefinition<{
            taskId: string;
            slug: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", TaskDocumentHistoryResponse, "taskDocumentsApi">;
    }, "TaskDocuments", "taskDocumentsApi">;
    browserApi: CombinedState<{
        browserStart: MutationDefinition<BrowserStartRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BROWSER", BrowserStartResponse, "browserApi">;
        browserStop: MutationDefinition<BrowserStopRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BROWSER", BrowserStopResponse, "browserApi">;
        browserScreenshot: MutationDefinition<BrowserScreenshotRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BROWSER", BrowserScreenshotResponse, "browserApi">;
        browserContext: MutationDefinition<BrowserContextRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BROWSER", BrowserContextResponse, "browserApi">;
        browserCurl: MutationDefinition<BrowserCurlRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BROWSER", BrowserCurlResponse, "browserApi">;
        browserElementPick: MutationDefinition<BrowserElementPickRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BROWSER", BrowserElementPickResponse, "browserApi">;
        browserElementPickResult: MutationDefinition<BrowserElementPickResultRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BROWSER", BrowserElementPickResultResponse, "browserApi">;
        browserRecordAnimation: MutationDefinition<BrowserRecordAnimationRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BROWSER", BrowserRecordAnimationResponse, "browserApi">;
        browserHandoff: MutationDefinition<BrowserHandoffRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BROWSER", BrowserHandoffResponse, "browserApi">;
        browserStatus: MutationDefinition<BrowserStatusRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BROWSER", BrowserStatusResponse, "browserApi">;
        browserAnnotateStart: MutationDefinition<BrowserAnnotateStartRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BROWSER", BrowserAnnotateStartResponse, "browserApi">;
        browserAnnotateResult: MutationDefinition<BrowserAnnotateResultRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BROWSER", BrowserAnnotateResultResponse, "browserApi">;
        browserAnnotateClear: MutationDefinition<BrowserAnnotateClearRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BROWSER", BrowserAnnotateClearResponse, "browserApi">;
        browserContextEstimate: MutationDefinition<BrowserContextEstimateRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BROWSER", BrowserContextEstimateResponse, "browserApi">;
        browserAction: MutationDefinition<BrowserActionRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BROWSER", BrowserActionResponse, "browserApi">;
    }, "BROWSER", "browserApi">;
    worktreesApi: CombinedState<{
        listWorktrees: QueryDefinition<{
            source_workspace_root?: string;
        } | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Worktrees", WorktreeListResponse, "worktreesApi">;
        getWorktreesSummary: QueryDefinition<{
            source_workspace_root?: string;
        } | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Worktrees", WorktreeInventory, "worktreesApi">;
        cleanupWorktreesDryRun: MutationDefinition<WorktreeCleanupRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Worktrees", WorktreeCleanupPlan, "worktreesApi">;
        cleanupWorktrees: MutationDefinition<WorktreeCleanupRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Worktrees", WorktreeCleanupResult, "worktreesApi">;
        createWorktree: MutationDefinition<CreateWorktreeRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Worktrees", CreateWorktreeResponse, "worktreesApi">;
        getWorktree: QueryDefinition<GetWorktreeRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Worktrees", WorktreeRecordView, "worktreesApi">;
        getWorktreeDiff: QueryDefinition<GetWorktreeDiffRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Worktrees", WorktreeDiffResponse, "worktreesApi">;
        mergeWorktree: MutationDefinition<MergeWorktreeRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Worktrees", MergeWorktreeResponse, "worktreesApi">;
        deleteWorktree: MutationDefinition<DeleteWorktreeRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Worktrees", DeleteWorktreeResponse, "worktreesApi">;
        openWorktree: MutationDefinition<GetWorktreeRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Worktrees", OpenWorktreeResponse, "worktreesApi">;
    }, "Worktrees", "worktreesApi">;
    skillsStatusApi: CombinedState<{
        getSkillsStatus: QueryDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, SkillsStatusResponse, "skillsStatusApi">;
    }, never, "skillsStatusApi">;
    mcpServerInfoApi: CombinedState<{
        getMCPServerInfo: QueryDefinition<{
            configPath: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MCPServerInfo", MCPServerInfo, "mcpServerInfoApi">;
        reconnectMCPServer: MutationDefinition<{
            configPath: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MCPServerInfo", {
            reconnect_triggered: boolean;
        }, "mcpServerInfoApi">;
    }, "MCPServerInfo", "mcpServerInfoApi">;
    chatModes: CombinedState<{
        getChatModes: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, ChatModesResponse, "chatModes">;
    }, never, "chatModes">;
    customizationApi: CombinedState<{
        getRegistry: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Registry" | "Config", RegistryResponse, "customizationApi">;
        getConfig: QueryDefinition<{
            kind: ConfigKind;
            id: string;
            scope?: "global" | "local";
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Registry" | "Config", ConfigDetailResponse, "customizationApi">;
        saveConfig: MutationDefinition<{
            kind: ConfigKind;
            id: string;
            config: Record<string, unknown>;
            scope?: "global" | "local";
            draft_id?: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Registry" | "Config", SaveConfigResponse, "customizationApi">;
        createConfig: MutationDefinition<{
            kind: ConfigKind;
            id: string;
            config: Record<string, unknown>;
            scope?: "global" | "local";
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Registry" | "Config", SaveConfigResponse, "customizationApi">;
        deleteConfig: MutationDefinition<{
            kind: ConfigKind;
            id: string;
            scope: "global" | "local";
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Registry" | "Config", DeleteConfigResponse, "customizationApi">;
    }, "Registry" | "Config", "customizationApi">;
    projectInformationApi: CombinedState<{
        getProjectInformation: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ProjectInformation", ProjectInformationConfig, "projectInformationApi">;
        saveProjectInformation: MutationDefinition<ProjectInformationConfig, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ProjectInformation", undefined, "projectInformationApi">;
        getProjectInformationPreview: MutationDefinition<ProjectInformationConfig, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ProjectInformation", ProjectInformationPreviewResponse, "projectInformationApi">;
    }, "ProjectInformation", "projectInformationApi">;
    setupStatus: CombinedState<{
        getSetupStatus: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, SetupStatusResponse, "setupStatus">;
    }, never, "setupStatus">;
    extensionsApi: CombinedState<{
        getExtRegistry: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtRegistry" | "Skill" | "Command" | "Hooks", ExtRegistryResponse, "extensionsApi">;
        getSkill: QueryDefinition<{
            name: string;
            scope?: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtRegistry" | "Skill" | "Command" | "Hooks", SkillDetail, "extensionsApi">;
        saveSkill: MutationDefinition<{
            name: string;
            scope?: string;
            body: Record<string, unknown>;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtRegistry" | "Skill" | "Command" | "Hooks", undefined, "extensionsApi">;
        createSkill: MutationDefinition<{
            name: string;
            scope: string;
            description: string;
            body: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtRegistry" | "Skill" | "Command" | "Hooks", undefined, "extensionsApi">;
        deleteSkill: MutationDefinition<{
            name: string;
            scope?: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtRegistry" | "Skill" | "Command" | "Hooks", undefined, "extensionsApi">;
        getCommand: QueryDefinition<{
            name: string;
            scope?: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtRegistry" | "Skill" | "Command" | "Hooks", CommandDetail, "extensionsApi">;
        saveCommand: MutationDefinition<{
            name: string;
            scope?: string;
            body: Record<string, unknown>;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtRegistry" | "Skill" | "Command" | "Hooks", undefined, "extensionsApi">;
        createCommand: MutationDefinition<Record<string, unknown>, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtRegistry" | "Skill" | "Command" | "Hooks", undefined, "extensionsApi">;
        deleteCommand: MutationDefinition<{
            name: string;
            scope?: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtRegistry" | "Skill" | "Command" | "Hooks", undefined, "extensionsApi">;
        getHooks: QueryDefinition<{
            scope?: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtRegistry" | "Skill" | "Command" | "Hooks", HooksDetail, "extensionsApi">;
        saveHooks: MutationDefinition<{
            scope?: string;
            body: Record<string, unknown>;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtRegistry" | "Skill" | "Command" | "Hooks", undefined, "extensionsApi">;
    }, "ExtRegistry" | "Skill" | "Command" | "Hooks", "extensionsApi">;
    pluginsApi: CombinedState<{
        getMarketplaces: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Marketplaces" | "InstalledPlugins", MarketplacesResponse, "pluginsApi">;
        addMarketplace: MutationDefinition<{
            source: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Marketplaces" | "InstalledPlugins", undefined, "pluginsApi">;
        deleteMarketplace: MutationDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Marketplaces" | "InstalledPlugins", undefined, "pluginsApi">;
        getMarketplacePlugins: QueryDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Marketplaces" | "InstalledPlugins", PluginListResponse, "pluginsApi">;
        installPlugin: MutationDefinition<{
            plugin: string;
            marketplace: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Marketplaces" | "InstalledPlugins", undefined, "pluginsApi">;
        getInstalled: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Marketplaces" | "InstalledPlugins", InstalledPluginsResponse, "pluginsApi">;
        uninstallPlugin: MutationDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Marketplaces" | "InstalledPlugins", undefined, "pluginsApi">;
    }, "Marketplaces" | "InstalledPlugins", "pluginsApi">;
    mcpMarketplaceApi: CombinedState<{
        getMarketplace: QueryDefinition<MarketplaceQueryParams | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", MarketplaceResponse, "mcpMarketplaceApi">;
        installServer: MutationDefinition<InstallRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", InstallResponse, "mcpMarketplaceApi">;
        getInstalledServers: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", InstalledResponse, "mcpMarketplaceApi">;
        getAutoName: MutationDefinition<AutoNameRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", AutoNameResponse, "mcpMarketplaceApi">;
        getMarketplaceSources: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", {
            sources: MarketplaceSource[];
        }, "mcpMarketplaceApi">;
        saveMarketplaceSource: MutationDefinition<SaveSourceRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", {
            ok: boolean;
        }, "mcpMarketplaceApi">;
        deleteMarketplaceSource: MutationDefinition<DeleteSourceRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", {
            ok: boolean;
        }, "mcpMarketplaceApi">;
        configureMarketplaceSource: MutationDefinition<ConfigureSourceRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", {
            ok: boolean;
        }, "mcpMarketplaceApi">;
    }, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", "mcpMarketplaceApi">;
    extensionsMarketplaceApi: CombinedState<{
        getExtensionMarketplaceSources: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", {
            sources: ExtensionMarketplaceSource[];
        }, "extensionsMarketplaceApi">;
        saveExtensionMarketplaceSource: MutationDefinition<{
            url: string;
            enabled: boolean;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", {
            ok: boolean;
            source: ExtensionMarketplaceSource;
        }, "extensionsMarketplaceApi">;
        deleteExtensionMarketplaceSource: MutationDefinition<{
            id: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", {
            ok: boolean;
        }, "extensionsMarketplaceApi">;
        configureExtensionMarketplaceSource: MutationDefinition<{
            id: string;
            enabled?: boolean;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", {
            ok: boolean;
        }, "extensionsMarketplaceApi">;
        getSubagentsMarketplace: QueryDefinition<{
            source?: string;
            q?: string;
        } | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", ExtensionMarketplaceResponse, "extensionsMarketplaceApi">;
        getSkillsMarketplace: QueryDefinition<{
            source?: string;
            q?: string;
        } | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", ExtensionMarketplaceResponse, "extensionsMarketplaceApi">;
        getCommandsMarketplace: QueryDefinition<{
            source?: string;
            q?: string;
        } | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", ExtensionMarketplaceResponse, "extensionsMarketplaceApi">;
        refreshExtensionMarketplaceSource: MutationDefinition<{
            id: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", {
            ok: boolean;
        }, "extensionsMarketplaceApi">;
        installMarketplaceSkill: MutationDefinition<{
            source_id: string;
            item_id: string;
            scope: "local" | "global";
            overwrite?: boolean;
            params?: Record<string, string>;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", ExtensionMarketplaceInstallResponse, "extensionsMarketplaceApi">;
        installMarketplaceCommand: MutationDefinition<{
            source_id: string;
            item_id: string;
            scope: "local" | "global";
            overwrite?: boolean;
            params?: Record<string, string>;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", ExtensionMarketplaceInstallResponse, "extensionsMarketplaceApi">;
        installMarketplaceSubagent: MutationDefinition<{
            source_id: string;
            item_id: string;
            scope: "local" | "global";
            overwrite?: boolean;
            params?: Record<string, string>;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", ExtensionMarketplaceInstallResponse, "extensionsMarketplaceApi">;
    }, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", "extensionsMarketplaceApi">;
    memoryEnrichmentApi: CombinedState<{
        previewMemoryEnrichment: MutationDefinition<{
            chatId: string;
            text: string;
            port: string | number;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, PreviewResult, "memoryEnrichmentApi">;
    }, never, "memoryEnrichmentApi">;
    buddyApi: CombinedState<{
        getBuddySnapshot: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BuddySnapshot" | "BuddyOpportunities" | "BuddyPulse" | "BuddyDrafts" | "BuddyArtifacts", BuddySnapshot, "buddyApi">;
        getBuddySettings: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BuddySnapshot" | "BuddyOpportunities" | "BuddyPulse" | "BuddyDrafts" | "BuddyArtifacts", BuddySettings, "buddyApi">;
        updateBuddySettings: MutationDefinition<Partial<BuddySettings> & {
            clear_personality_prompt?: boolean;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BuddySnapshot" | "BuddyOpportunities" | "BuddyPulse" | "BuddyDrafts" | "BuddyArtifacts", BuddySettingsResponse, "buddyApi">;
        careBuddy: MutationDefinition<BuddyCareRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BuddySnapshot" | "BuddyOpportunities" | "BuddyPulse" | "BuddyDrafts" | "BuddyArtifacts", BuddyCareResponse, "buddyApi">;
        acceptBuddyQuest: MutationDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BuddySnapshot" | "BuddyOpportunities" | "BuddyPulse" | "BuddyDrafts" | "BuddyArtifacts", BuddyQuestAcceptResponse, "buddyApi">;
        rerollBuddyPersonality: MutationDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BuddySnapshot" | "BuddyOpportunities" | "BuddyPulse" | "BuddyDrafts" | "BuddyArtifacts", BuddyPersonalityRerollResponse, "buddyApi">;
        getBuddyActivities: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BuddySnapshot" | "BuddyOpportunities" | "BuddyPulse" | "BuddyDrafts" | "BuddyArtifacts", BuddyActivityEntry[], "buddyApi">;
        getBuddyConversations: QueryDefinition<{
            kind?: string;
        } | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BuddySnapshot" | "BuddyOpportunities" | "BuddyPulse" | "BuddyDrafts" | "BuddyArtifacts", BuddyConversationEntry[], "buddyApi">;
        createBuddyConversation: MutationDefinition<BuddyConversationCreateRequest | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BuddySnapshot" | "BuddyOpportunities" | "BuddyPulse" | "BuddyDrafts" | "BuddyArtifacts", BuddyConversationMeta, "buddyApi">;
        createSetupConversation: MutationDefinition<{
            flow: string;
            title?: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BuddySnapshot" | "BuddyOpportunities" | "BuddyPulse" | "BuddyDrafts" | "BuddyArtifacts", {
            chat_id: string;
            title: string;
            kind: string;
            flow: string;
            badge: string;
            created_at: string;
        }, "buddyApi">;
        dismissBuddySuggestion: MutationDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BuddySnapshot" | "BuddyOpportunities" | "BuddyPulse" | "BuddyDrafts" | "BuddyArtifacts", {
            dismissed: boolean;
        }, "buddyApi">;
        dismissBuddyRuntimeEvent: MutationDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BuddySnapshot" | "BuddyOpportunities" | "BuddyPulse" | "BuddyDrafts" | "BuddyArtifacts", {
            dismissed: boolean;
        }, "buddyApi">;
        reportError: MutationDefinition<BuddyErrorReport, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BuddySnapshot" | "BuddyOpportunities" | "BuddyPulse" | "BuddyDrafts" | "BuddyArtifacts", null, "buddyApi">;
        getOpportunities: QueryDefinition<{
            status?: OpportunityStatus;
        } | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BuddySnapshot" | "BuddyOpportunities" | "BuddyPulse" | "BuddyDrafts" | "BuddyArtifacts", BuddyOpportunity[], "buddyApi">;
        acceptOpportunity: MutationDefinition<{
            id: string;
            action_index: number;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BuddySnapshot" | "BuddyOpportunities" | "BuddyPulse" | "BuddyDrafts" | "BuddyArtifacts", BuddyOpportunityAcceptResponse, "buddyApi">;
        dismissOpportunity: MutationDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BuddySnapshot" | "BuddyOpportunities" | "BuddyPulse" | "BuddyDrafts" | "BuddyArtifacts", BuddyOpportunityDismissResponse, "buddyApi">;
        getPulse: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BuddySnapshot" | "BuddyOpportunities" | "BuddyPulse" | "BuddyDrafts" | "BuddyArtifacts", BuddyPulse, "buddyApi">;
        createSkillDraft: MutationDefinition<CreateDraftRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BuddySnapshot" | "BuddyOpportunities" | "BuddyPulse" | "BuddyDrafts" | "BuddyArtifacts", BuddyDraft, "buddyApi">;
        createCommandDraft: MutationDefinition<CreateDraftRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BuddySnapshot" | "BuddyOpportunities" | "BuddyPulse" | "BuddyDrafts" | "BuddyArtifacts", BuddyDraft, "buddyApi">;
        createSubagentDraft: MutationDefinition<CreateDraftRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BuddySnapshot" | "BuddyOpportunities" | "BuddyPulse" | "BuddyDrafts" | "BuddyArtifacts", BuddyDraft, "buddyApi">;
        createModeDraft: MutationDefinition<CreateDraftRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BuddySnapshot" | "BuddyOpportunities" | "BuddyPulse" | "BuddyDrafts" | "BuddyArtifacts", BuddyDraft, "buddyApi">;
        createAgentsMdDraft: MutationDefinition<CreateDraftRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BuddySnapshot" | "BuddyOpportunities" | "BuddyPulse" | "BuddyDrafts" | "BuddyArtifacts", BuddyDraft, "buddyApi">;
        createDefaultsDraft: MutationDefinition<CreateDraftRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BuddySnapshot" | "BuddyOpportunities" | "BuddyPulse" | "BuddyDrafts" | "BuddyArtifacts", BuddyDraft, "buddyApi">;
        createHookDraft: MutationDefinition<CreateDraftRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BuddySnapshot" | "BuddyOpportunities" | "BuddyPulse" | "BuddyDrafts" | "BuddyArtifacts", BuddyDraft, "buddyApi">;
        createPulseReportDraft: MutationDefinition<CreateDraftRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BuddySnapshot" | "BuddyOpportunities" | "BuddyPulse" | "BuddyDrafts" | "BuddyArtifacts", BuddyDraft, "buddyApi">;
        getDraft: QueryDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BuddySnapshot" | "BuddyOpportunities" | "BuddyPulse" | "BuddyDrafts" | "BuddyArtifacts", BuddyDraft, "buddyApi">;
        deleteDraft: MutationDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BuddySnapshot" | "BuddyOpportunities" | "BuddyPulse" | "BuddyDrafts" | "BuddyArtifacts", undefined, "buddyApi">;
        postUserAction: MutationDefinition<UserActionPayload, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BuddySnapshot" | "BuddyOpportunities" | "BuddyPulse" | "BuddyDrafts" | "BuddyArtifacts", undefined, "buddyApi">;
        getUserActivity: QueryDefinition<{
            hours?: number;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BuddySnapshot" | "BuddyOpportunities" | "BuddyPulse" | "BuddyDrafts" | "BuddyArtifacts", UserActivityResponse, "buddyApi">;
        reportFrontendError: MutationDefinition<FrontendErrorReport, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BuddySnapshot" | "BuddyOpportunities" | "BuddyPulse" | "BuddyDrafts" | "BuddyArtifacts", null, "buddyApi">;
        getBuddyArtifacts: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BuddySnapshot" | "BuddyOpportunities" | "BuddyPulse" | "BuddyDrafts" | "BuddyArtifacts", MemoryOpsState, "buddyApi">;
        approveBuddyArtifact: MutationDefinition<{
            op_id: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BuddySnapshot" | "BuddyOpportunities" | "BuddyPulse" | "BuddyDrafts" | "BuddyArtifacts", undefined, "buddyApi">;
        rejectBuddyArtifact: MutationDefinition<{
            op_id: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BuddySnapshot" | "BuddyOpportunities" | "BuddyPulse" | "BuddyDrafts" | "BuddyArtifacts", undefined, "buddyApi">;
    }, "BuddySnapshot" | "BuddyOpportunities" | "BuddyPulse" | "BuddyDrafts" | "BuddyArtifacts", "buddyApi">;
} & PersistPartial, undefined, UnknownAction> & Dispatch<Action>>;
