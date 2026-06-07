import React from "react";
type UsageCounterProps = {
    isInline?: boolean;
    isMessageEmpty?: boolean;
} | {
    isInline: true;
    isMessageEmpty: boolean;
};
export declare const UsageCounter: React.FC<UsageCounterProps>;
export {};
