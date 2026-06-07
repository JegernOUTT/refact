import React from "react";
import { type ChatLink } from "../../services/refact/links";
export declare const ChatLinks: React.FC;
export declare const ChatLinkButton: React.FC<{
    link: ChatLink;
    onClick: (link: ChatLink) => void;
}>;
