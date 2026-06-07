import React from "react";
type ProviderOAuthProps = {
    providerName: string;
    baseProvider?: string;
    oauthConnected: boolean;
    authStatus: string;
};
export declare const ProviderOAuth: React.FC<ProviderOAuthProps>;
export {};
