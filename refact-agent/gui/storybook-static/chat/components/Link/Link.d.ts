import { FC } from "react";
import { type LinkProps as RadixLinkProps } from "@radix-ui/themes";
interface LinkProps extends RadixLinkProps {
    href?: string;
    children?: React.ReactNode;
    className?: string;
    onClick?: React.MouseEventHandler<HTMLAnchorElement>;
}
export declare const Link: FC<LinkProps>;
export {};
