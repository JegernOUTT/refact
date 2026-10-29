import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import classNames from "classnames";
import markdownStyles from "../../components/Markdown/Markdown.module.css";
import { fenceMachinePayloads } from "./BuddyArtifactBodyText";
import { humanizeBuddyKey } from "./ui";
import styles from "./BuddyArtifactBody.module.css";

const plugins = [remarkGfm];

const MACHINE_HEADING = /^[a-z0-9]+(?:_[a-z0-9]+)+$/;

const heading: Components["h4"] = ({ children }) => (
  <h4 className={styles.heading}>
    {typeof children === "string" && MACHINE_HEADING.test(children.trim())
      ? humanizeBuddyKey(children)
      : children}
  </h4>
);

const components: Components = {
  a: ({ children }) => <span className={styles.inertLink}>{children}</span>,
  h1: heading,
  h2: heading,
  h3: heading,
  h4: heading,
  h5: heading,
  h6: heading,
  ul: ({ children }) => <ul className={markdownStyles.list}>{children}</ul>,
  ol: ({ children }) => <ol className={markdownStyles.list}>{children}</ol>,
  li: ({ children }) => (
    <li className={markdownStyles.list_item}>{children}</li>
  ),
  code: ({ className, children }) =>
    className ? (
      <code className={className}>{children}</code>
    ) : (
      <code className={markdownStyles.code_inline}>{children}</code>
    ),
  img: ({ alt }) => (
    <span>{alt ? `Image omitted: ${alt}` : "Image omitted"}</span>
  ),
  input: ({ checked }) => <span>{checked ? "[x]" : "[ ]"}</span>,
  pre: ({ children }) => <pre className={styles.code}>{children}</pre>,
  table: ({ children }) => (
    <div className={markdownStyles.tableScroll}>
      <table className={markdownStyles.table}>{children}</table>
    </div>
  ),
};

export function BuddyArtifactBody({ body }: { body: string }) {
  return (
    <div className={classNames(markdownStyles.markdown, styles.body)}>
      <ReactMarkdown remarkPlugins={plugins} skipHtml components={components}>
        {fenceMachinePayloads(body)}
      </ReactMarkdown>
    </div>
  );
}
