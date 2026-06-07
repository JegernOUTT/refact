import { FC, useCallback } from "react";
import classNames from "classnames";
import { useConfig, useOpenUrl } from "../../hooks";
import styles from "./Link.module.css";

interface LinkProps extends React.ComponentPropsWithoutRef<"a"> {
  href?: string;
  children?: React.ReactNode;
  className?: string;
  onClick?: React.MouseEventHandler<HTMLAnchorElement>;
  size?: string;
}

export const Link: FC<LinkProps> = ({ onClick, size: _size, ...props }) => {
  const config = useConfig();
  const openUrl = useOpenUrl();

  const href = props.href ?? "";
  const isExternalUrl =
    href.startsWith("http://") || href.startsWith("https://");

  const handleClick: React.MouseEventHandler<HTMLAnchorElement> = useCallback(
    (e) => {
      if (onClick) {
        onClick(e);
      }
      if (config.host === "jetbrains" && isExternalUrl && !e.defaultPrevented) {
        e.preventDefault();
        openUrl(href);
      }
    },
    [onClick, config.host, isExternalUrl, openUrl, href],
  );

  return (
    <a
      className={classNames(
        styles.link,
        { [styles.jetbrains]: config.host === "jetbrains" },
        props.className,
      )}
      onClick={handleClick}
      {...props}
    />
  );
};
