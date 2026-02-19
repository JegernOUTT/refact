import React from "react";
import { useStoredOpen } from "./useStoredOpen";
import { Container, Box, Flex, Text } from "@radix-ui/themes";
import { Markdown } from "./ContextFiles";
import styles from "./ChatContent.module.css";
import { ScrollArea } from "../ScrollArea";
import {
  FileTextIcon,
  ChevronDownIcon,
  ChevronRightIcon,
} from "@radix-ui/react-icons";
import * as Collapsible from "@radix-ui/react-collapsible";

export type PlainTextProps = {
  children: string;
  id?: string;
  defaultOpen?: boolean;
};

export const PlainText: React.FC<PlainTextProps> = ({
  children,
  id,
  defaultOpen = false,
}) => {
  const storeKey = id ? `plaintext:${id}` : undefined;
  const [open, _toggleOpen, setOpen] = useStoredOpen(storeKey, defaultOpen);
  const text = "```text\n" + children + "\n```";
  const preview =
    children.slice(0, 100).replace(/\n/g, " ") +
    (children.length > 100 ? "..." : "");

  return (
    <Container position="relative" data-plain-text-id={id}>
      <Collapsible.Root open={open} onOpenChange={setOpen}>
        <Collapsible.Trigger asChild>
          <Flex
            gap="2"
            align="center"
            py="1"
            className={styles.plainTextTrigger}
          >
            <FileTextIcon width="14" height="14" />
            <Text size="1" weight="light" style={{ color: "var(--gray-10)" }}>
              Plain text
            </Text>
            <Text size="1" style={{ color: "var(--gray-9)", flex: 1 }} truncate>
              {preview}
            </Text>
            {open ? (
              <ChevronDownIcon width="14" height="14" />
            ) : (
              <ChevronRightIcon width="14" height="14" />
            )}
          </Flex>
        </Collapsible.Trigger>
        <Collapsible.Content>
          <ScrollArea scrollbars="both">
            <Box style={{ maxHeight: "300px" }} pl="4">
              <Markdown>{text}</Markdown>
            </Box>
          </ScrollArea>
        </Collapsible.Content>
      </Collapsible.Root>
    </Container>
  );
};
