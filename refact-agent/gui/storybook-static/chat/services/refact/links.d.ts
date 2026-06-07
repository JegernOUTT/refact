import { reactHooksModuleName } from '@reduxjs/toolkit/query/react';
import { Api, BaseQueryFn, FetchArgs, FetchBaseQueryError, FetchBaseQueryMeta, QueryDefinition, MutationDefinition, coreModuleName } from '@reduxjs/toolkit/query';
import { ChatMessage, ChatMessages } from "./types";
import { LspChatMode } from "../../features/Chat";
type LinkActions = "patch-all" | "follow-up" | "commit" | "goto" | "post-chat" | "regenerate-with-increased-context-size";
export type ChatLink = BaseLink | CommitLink | PostChatLink;
interface BaseLink {
    link_action: LinkActions;
    link_text: string;
    link_goto?: string;
    link_tooltip?: string;
    link_payload?: CommitLinkPayload | PostChatLinkPayload | null;
    link_summary_path?: string;
}
export interface CommitLink extends BaseLink {
    link_text: string;
    link_action: "commit";
    link_goto: string;
    link_tooltip: string;
    link_payload: CommitLinkPayload;
}
export interface PostChatLink extends BaseLink {
    link_action: "post-chat";
    link_payload: PostChatLinkPayload;
}
export type CommitLinkPayload = {
    project_path: string;
    commit_message: string;
    file_changes: {
        path: string;
        status: string;
    }[];
};
export type PostChatLinkPayload = {
    chat_meta: {
        chat_id: string;
        chat_remote: boolean;
        chat_mode: "CONFIGURE";
        current_config_file: string;
    };
    messages: ChatMessage[];
};
export declare function isCommitLink(chatLink: ChatLink): chatLink is CommitLink;
export declare function isPostChatLink(chatLink: ChatLink): chatLink is PostChatLink;
export type LinksForChatResponse = {
    links: ChatLink[];
    uncommited_changes_warning: string;
    new_chat_suggestion: boolean;
};
export type LinksApiRequest = {
    chat_id: string;
    messages: ChatMessages;
    model: string;
    mode?: LspChatMode;
    current_config_file?: string;
};
export declare const linksApi: Api<BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, {
    getLinksForChat: QueryDefinition<LinksApiRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Chat_Links", LinksForChatResponse, "linksApi">;
    sendCommit: MutationDefinition<CommitLinkPayload, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "Chat_Links", CommitResponse, "linksApi">;
}, "linksApi", "Chat_Links", typeof coreModuleName | typeof reactHooksModuleName>;
export type CommitResponse = {
    commits_applied: {
        project_path: string;
        project_name: string;
        commit_oid: string;
    }[];
    error_log: {
        error_message: string;
        project_path: string;
        project_name: string;
    }[];
};
export {};
