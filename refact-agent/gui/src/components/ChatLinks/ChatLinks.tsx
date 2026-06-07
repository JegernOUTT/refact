import React from "react";
import { LoaderCircle } from "lucide-react";
import { Button, Icon } from "../ui";
import { type ChatLink } from "../../services/refact/links";
import { useAppSelector, useLinksFromLsp } from "../../hooks";
import { TruncateRight } from "../Text/TruncateRight";

import styles from "./ChatLinks.module.css";
import { selectAreFollowUpsEnabled } from "../../features/Chat";

function maybeConcatActionAndGoToStrings(link: ChatLink): string | undefined {
  const hasAction = "link_action" in link;
  const hasGoTo = "link_goto" in link;
  if (!hasAction && !hasGoTo) return "";
  if (hasAction && hasGoTo) {
    return `action: ${link.link_action}\ngoto: ${link.link_goto}`;
  }
  return `action: ${link.link_action}`;
}

export const ChatLinks: React.FC = () => {
  const { linksResult, handleLinkAction, streaming } = useLinksFromLsp();
  const areFollowUpsEnabled = useAppSelector(selectAreFollowUpsEnabled);
  if (streaming || !areFollowUpsEnabled) return null;

  // TODO: waiting, errors, maybe add a title

  if (linksResult.isLoading || linksResult.isFetching) {
    return (
      <Button variant="soft" disabled>
        <span className={styles.spinner} aria-hidden="true">
          <Icon icon={LoaderCircle} size="sm" />
        </span>
        Checking for actions
      </Button>
    );
  }

  if (linksResult.data && linksResult.data.links.length > 0) {
    return linksResult.data.links.map((link, index) => {
      const key = `chat-link-${index}`;
      return (
        <ChatLinkButton key={key} link={link} onClick={handleLinkAction} />
      );
    });
  }

  return null;
};

export const ChatLinkButton: React.FC<{
  link: ChatLink;
  onClick: (link: ChatLink) => void;
}> = ({ link, onClick }) => {
  const title = link.link_tooltip ?? maybeConcatActionAndGoToStrings(link);

  const handleClick = React.useCallback(() => onClick(link), [link, onClick]);
  return (
    <Button
      variant="soft"
      title={title}
      onClick={handleClick}
      className={styles.chat_link_button}
    >
      <TruncateRight>{link.link_text}</TruncateRight>
    </Button>
  );
};
