export type RTKResponseErrorWithDetailMessage = {
    error: {
        data: {
            detail: string;
        };
    };
};
export declare function isRTKResponseErrorWithDetailMessage(json: unknown): json is RTKResponseErrorWithDetailMessage;
