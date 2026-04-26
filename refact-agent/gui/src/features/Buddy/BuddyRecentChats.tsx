import React, { useCallback } from "react";
import { Flex, Text, Button, Spinner } from "@radix-ui/themes";
import { ChatBubbleIcon, PlusIcon } from "@radix-ui/react-icons";
import { useAppDispatch } from "../../hooks";
import { push } from "../Pages/pagesSlice";
import { openBuddyChat, newBuddyChatAction } from "../Chat/Thread";
import {
  useGetBuddyConversationsQuery,
  useCreateBuddyConversationMutation,
} from "../../services/refact/buddy";
import styles from "./BuddyHome.module.css";

export const BuddyRecentChats: React.FC = () => {
  const dispatch = useAppDispatch();
  const { data: conversations, isLoading } = useGetBuddyConversationsQuery(
    undefined,
    { refetchOnMountOrArgChange: true },
  );
  const [createConversation, { isLoading: isCreating }] =
    useCreateBuddyConversationMutation();

  const handleOpen = useCallback(
    (chatId: string, title: string) => {
      dispatch(openBuddyChat({ chat_id: chatId, title }));
      dispatch(push({ name: "chat" }));
    },
    [dispatch],
  );

  const handleNew = useCallback(async () => {
    const result = await createConversation(undefined);
    if ("data" in result && result.data) {
      const meta = result.data;
      dispatch(newBuddyChatAction({ chat_id: meta.chat_id }));
      dispatch(openBuddyChat({ chat_id: meta.chat_id, title: meta.title }));
      dispatch(push({ name: "chat" }));
    }
  }, [createConversation, dispatch]);

  return (
    <Flex direction="column" gap="2">
      <Flex align="center" justify="between">
        <Text size="1" weight="bold" color="gray" className={styles.sectionLabel}>
          RECENT CHATS
        </Text>
        <Button
          size="1"
          variant="ghost"
          onClick={handleNew}
          disabled={isCreating}
        >
          {isCreating ? (
            <Spinner size="1" />
          ) : (
            <PlusIcon width={12} height={12} />
          )}
          New Chat
        </Button>
      </Flex>

      {isLoading && (
        <Flex align="center" justify="center" py="3">
          <Spinner size="2" />
        </Flex>
      )}

      {!isLoading && (!conversations || conversations.length === 0) && (
        <Flex
          direction="column"
          align="center"
          justify="center"
          gap="2"
          py="4"
          className={styles.emptyChats}
        >
          <ChatBubbleIcon width={20} height={20} />
          <Text size="1" color="gray">
            No buddy chats yet
          </Text>
          <Button size="1" variant="soft" onClick={handleNew}>
            Start a conversation
          </Button>
        </Flex>
      )}

      {conversations &&
        conversations.map((conv) => (
          <button
            key={conv.chat_id}
            type="button"
            className={styles.chatItem}
            onClick={() => handleOpen(conv.chat_id, conv.title)}
          >
            <Flex align="center" gap="2">
              <ChatBubbleIcon
                width={13}
                height={13}
                style={{ flexShrink: 0, opacity: 0.6 }}
              />
              <Flex direction="column" gap="0" style={{ flex: 1, minWidth: 0 }}>
                <Text
                  size="1"
                  weight="medium"
                  className={styles.chatTitle}
                  truncate
                >
                  {conv.title || "Buddy Chat"}
                </Text>
                {conv.last_message_at && (
                  <Text size="1" color="gray" className={styles.chatTime}>
                    {new Date(conv.last_message_at).toLocaleDateString()}
                  </Text>
                )}
              </Flex>
              <Text size="1" color="gray" style={{ flexShrink: 0 }}>
                {conv.message_count > 0 ? `${conv.message_count}` : ""}
              </Text>
            </Flex>
          </button>
        ))}
    </Flex>
  );
};
