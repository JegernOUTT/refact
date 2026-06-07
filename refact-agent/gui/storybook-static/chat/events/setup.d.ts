export declare enum EVENT_NAMES_FROM_SETUP {
    OPEN_EXTERNAL_URL = "open_external_url"
}
export interface ActionFromSetup {
    type: EVENT_NAMES_FROM_SETUP;
    payload?: Record<string, unknown>;
}
export declare function isActionFromSetup(action: unknown): action is ActionFromSetup;
export interface OpenExternalUrl extends ActionFromSetup {
    type: EVENT_NAMES_FROM_SETUP.OPEN_EXTERNAL_URL;
    payload: {
        url: string;
    };
}
export declare function isOpenExternalUrl(action: unknown): action is OpenExternalUrl;
