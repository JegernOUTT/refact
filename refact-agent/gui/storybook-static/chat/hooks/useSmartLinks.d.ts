import { LspChatMessage } from "../services/refact/chat";
export declare function useSmartLinks(): {
    handleSmartLink: (sl_chat: LspChatMessage[], integrationName: string, integrationPath: string, integrationProject: string) => void;
    handleGoTo: ({ goto }: {
        goto?: string;
    }) => void;
};
