import { FC, useCallback } from "react";
import classNames from "classnames";
import { useConfig } from "../../hooks/useConfig";
import { useOpenUrl } from "../../hooks/useOpenUrl";
import {
  contentRouteHref,
  isPrimaryContentClick,
  routeFromContentLink,
} from "../../utils/contentRoutes";
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

  const route = routeFromContentLink(props.href ?? "");
  const href =
    config.host === "web" && route ? contentRouteHref(route) : props.href ?? "";
  const isExternalUrl =
    href.startsWith("http://") || href.startsWith("https://");

  const handleClick: React.MouseEventHandler<HTMLAnchorElement> = useCallback(
    (e) => {
      if (
        config.host === "web" &&
        route &&
        (!isPrimaryContentClick(e) ||
          (props.target && props.target !== "_self"))
      )
        return;
      if (onClick) onClick(e);
      if (config.host === "jetbrains" && isExternalUrl && !e.defaultPrevented) {
        e.preventDefault();
        openUrl(href);
      }
    },
    [onClick, config.host, isExternalUrl, openUrl, href, route, props.target],
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
      href={href}
      rel={props.target === "_blank" ? "noopener noreferrer" : props.rel}
    />
  );
};
