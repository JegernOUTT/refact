import React from "react";
import { type BoxProps } from "@radix-ui/themes";
import { type ScrollAreaProps } from "./ScrollArea";
/**
 * Check list
 * Static chat
 * ✅ When give a long chat it should start from the last user message
 * ✅ When clicking the follow button it should go to the bottom of the screen
 * ✅ When at the bottom the follow button should not show
 *
 * In progress chat.
 * ✅ When a user message is submitted it should go to the user message
 * ✅ When i click the follow button it should follow the chat
 *
 */
export declare const ScrollArea: React.ForwardRefExoticComponent<Omit<ScrollAreaProps, "ref"> & React.RefAttributes<HTMLDivElement>>;
export type ScrollAnchorProps = React.PropsWithChildren<ScrollIntoViewOptions & BoxProps>;
export declare const ScrollAnchor: React.FC<ScrollAnchorProps>;
