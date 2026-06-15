import React from "react";
import ReactMarkdown, {
  defaultUrlTransform,
  type UrlTransform,
} from "react-markdown";
import styles from "./Command.module.css";
import classNames from "classnames";
import { ShikiCodeBlock } from "../Markdown/ShikiCodeBlock";

const dataUrlPattern =
  /^data:image\/(png|jpeg|gif|bmp|webp);base64,[A-Za-z0-9+/]+={0,2}$/;

const urlTransform: UrlTransform = (value) => {
  if (dataUrlPattern.test(value)) {
    return value;
  }
  return defaultUrlTransform(value);
};

export type MarkdownProps = {
  children: string;
  className?: string;
  isInsideScrollArea?: boolean;
  variant?: "chat" | "tool" | "terminal";
};

const Image: React.FC<
  React.DetailedHTMLProps<
    React.ImgHTMLAttributes<HTMLImageElement>,
    HTMLImageElement
  >
> = ({ ...props }) => {
  return <img {...props} className={styles.image} />;
};

export const Markdown: React.FC<MarkdownProps> = ({
  children,
  className,
  isInsideScrollArea,
  variant = "terminal",
}) => {
  return (
    <ReactMarkdown
      urlTransform={urlTransform}
      className={classNames(
        styles.markdown,
        styles[`variant_${variant}`],
        className,
        {
          [styles.isInsideScrollArea]: isInsideScrollArea,
        },
      )}
      components={{
        code({ color: _color, ref: _ref, node: _node, ...props }) {
          return <ShikiCodeBlock {...props} />;
        },
        p({ color: _color, ref: _ref, node: _node, ...props }) {
          return <div {...props} />;
        },
        img({ color: _color, ref: _ref, node: _node, ...props }) {
          return <Image {...props} />;
        },
      }}
    >
      {children}
    </ReactMarkdown>
  );
};

export type CommandMarkdownProps = MarkdownProps;
export const CommandMarkdown: React.FC<CommandMarkdownProps> = (props) => (
  <Markdown {...props} />
);
