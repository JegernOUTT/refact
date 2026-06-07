import React from "react";
import { IconButton, Button, Flex } from "@radix-ui/themes";
type IconButtonProps = React.ComponentProps<typeof IconButton>;
type ButtonProps = React.ComponentProps<typeof Button>;
export declare const PaperPlaneButton: React.FC<IconButtonProps>;
type PlainButtonProps = React.ButtonHTMLAttributes<HTMLButtonElement>;
export declare const AgentIntegrationsButton: React.ForwardRefExoticComponent<PlainButtonProps & React.RefAttributes<HTMLButtonElement>>;
export declare const ThreadHistoryButton: React.FC<IconButtonProps>;
export declare const BackToSideBarButton: React.FC<PlainButtonProps>;
export declare const CloseButton: React.FC<IconButtonProps & {
    iconSize?: number | string;
}>;
export declare const RightButton: React.FC<ButtonProps & {
    className?: string;
}>;
type FlexProps = React.ComponentProps<typeof Flex>;
export declare const RightButtonGroup: React.FC<React.PropsWithChildren & FlexProps>;
export {};
