import type { FC } from "react";
type IntegrationAvailabilityProps = {
    fieldName: string;
    value: boolean;
    onChange: (fieldName: string, value: boolean) => void;
};
export declare const IntegrationAvailability: FC<IntegrationAvailabilityProps>;
export {};
