import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import styles from "./BuddyArtifactBody.module.css";

const plugins = [remarkGfm];

export function BuddyArtifactBody({ body }: { body: string }) {
  return (
    <div className={styles.body}>
      <ReactMarkdown
        remarkPlugins={plugins}
        skipHtml
        components={{
          a: ({ children }) => <span>{children}</span>,
          img: ({ alt }) => (
            <span>{alt ? `Image omitted: ${alt}` : "Image omitted"}</span>
          ),
          input: ({ checked }) => <span>{checked ? "[x]" : "[ ]"}</span>,
          pre: ({ children }) => <pre className={styles.code}>{children}</pre>,
          table: ({ children }) => (
            <div className="scrollX">
              <table>{children}</table>
            </div>
          ),
        }}
      >
        {body}
      </ReactMarkdown>
    </div>
  );
}
