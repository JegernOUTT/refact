import { type MouseEventHandler } from "react";
import { ProviderCardProps } from "./ProviderCard";
export declare function useProviderCard({ provider, setCurrentProvider, }: {
    provider: ProviderCardProps["provider"];
    setCurrentProvider: ProviderCardProps["setCurrentProvider"];
}): {
    handleClickOnProvider: () => void;
    handleSwitchClick: MouseEventHandler<HTMLDivElement>;
};
