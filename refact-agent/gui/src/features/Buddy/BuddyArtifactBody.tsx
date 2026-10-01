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
          h1: ({ children }) => <h4>{children}</h4>,
          h2: ({ children }) => <h5>{children}</h5>,
          h3: ({ children }) => <h6>{children}</h6>,
          h4: ({ children }) => <h6>{children}</h6>,
          h5: ({ children }) => <h6>{children}</h6>,
          h6: ({ children }) => <h6>{children}</h6>,
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
