import React from "react";
import { Card, Flex, Text, ScrollArea, Badge } from "@radix-ui/themes";
import { useGetChatModesQuery, ChatModeInfo } from "../../services/refact/chatModes";
import { DEFAULT_MODE } from "../../features/Chat/Thread/types";

type ModeSelectProps = {
  selectedMode: string;
  onModeChange: (modeId: string) => void;
};

export const ModeSelect: React.FC<ModeSelectProps> = ({
  selectedMode,
  onModeChange,
}) => {
  const { data, isLoading, isError } = useGetChatModesQuery();

  const modes = data?.modes ?? [];
  const hasErrors = (data?.errors?.length ?? 0) > 0;

  const effectiveMode = selectedMode || DEFAULT_MODE;

  if (isLoading) {
    return (
      <Flex direction="column" gap="2" mb="3">
        <Text size="2" color="gray">Loading modes...</Text>
      </Flex>
    );
  }

  if (isError) {
    return (
      <Flex direction="column" gap="2" mb="3">
        <Badge color="red" size="1">Failed to load modes</Badge>
      </Flex>
    );
  }

  if (modes.length === 0) {
    return (
      <Flex direction="column" gap="2" mb="3">
        <Text size="2" color="gray">No modes configured</Text>
      </Flex>
    );
  }

  return (
    <Flex direction="column" gap="2" mb="3">
      <Flex justify="between" align="center">
        <Text size="2">Select mode:</Text>
        {hasErrors && (
          <Badge color="orange" size="1">Config errors</Badge>
        )}
      </Flex>
      <ScrollArea scrollbars="horizontal" style={{ maxWidth: "100%" }}>
        <Flex gap="2" pb="2">
          {modes.map((mode) => (
            <ModeCard
              key={mode.id}
              mode={mode}
              isSelected={effectiveMode === mode.id}
              onClick={() => onModeChange(mode.id)}
            />
          ))}
        </Flex>
      </ScrollArea>
    </Flex>
  );
};

type ModeCardProps = {
  mode: ChatModeInfo;
  isSelected: boolean;
  onClick: () => void;
};

const ModeCard: React.FC<ModeCardProps> = ({ mode, isSelected, onClick }) => {
  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      onClick();
    }
  };

  return (
    <Card
      size="1"
      role="button"
      tabIndex={0}
      onKeyDown={handleKeyDown}
      style={{
        minWidth: 140,
        maxWidth: 180,
        cursor: "pointer",
        outline: isSelected ? "2px solid var(--accent-9)" : "1px solid var(--gray-6)",
        outlineOffset: -1,
        backgroundColor: isSelected ? "var(--accent-3)" : undefined,
      }}
      onClick={onClick}
    >
      <Flex direction="column" gap="1">
        <Text size="2" weight="bold" truncate>
          {mode.title}
        </Text>
        <Text size="1" color="gray" style={{ minHeight: 32 }}>
          {mode.description.slice(0, 60)}{mode.description.length > 60 ? "..." : ""}
        </Text>
        <Flex gap="1" wrap="wrap">
          {mode.ui.tags.slice(0, 2).map((tag) => (
            <Badge key={tag} size="1" color="gray" variant="soft">
              {tag}
            </Badge>
          ))}
          {mode.tools_count > 0 && (
            <Badge size="1" color="blue" variant="soft">
              {mode.tools_count} tools
            </Badge>
          )}
        </Flex>
      </Flex>
    </Card>
  );
};

ModeSelect.displayName = "ModeSelect";
