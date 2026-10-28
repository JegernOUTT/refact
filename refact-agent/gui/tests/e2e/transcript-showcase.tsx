import React from "react";
import { createRoot } from "react-dom/client";
import { Provider } from "react-redux";
import {
  Ban,
  Bot,
  FilePen,
  FileText,
  Globe,
  ListChecks,
  MessageCircleQuestion,
  Plug,
  Search,
  Send,
  ShieldCheck,
  Terminal,
  TriangleAlert,
  Users,
} from "lucide-react";
import { setUpStore } from "../../src/app/store";
import { Theme } from "../../src/components/Theme";
import { Button } from "../../src/components/ui";
import {
  TranscriptClamp,
  TranscriptItem,
} from "../../src/components/ChatContent/TranscriptItem";
import { ToolCard } from "../../src/components/ChatContent/ToolCard/ToolCard";
import { ToolCard as KitToolCard } from "../../src/components/ui";
import "../../src/lib/render/web.css";
import "./transcript-showcase.css";

const execLines = Array.from(
  { length: 80 },
  (_, i) => `test result ${i + 1}: ok`,
).join("\n");

function Diff() {
  return (
    <pre>
      <span className="diff-del">- const limit = 10;</span>
      {"\n"}
      <span className="diff-add">+ const limit = LIMIT_DEFAULT;</span>
    </pre>
  );
}

function LegacyWrapperCard() {
  const [open, setOpen] = React.useState(false);
  return (
    <ToolCard
      icon={<Terminal />}
      isOpen={open}
      meta="legacy wrapper"
      status="success"
      summary={<code>ls -la</code>}
      onToggle={() => setOpen((value) => !value)}
    >
      <pre>total 0</pre>
    </ToolCard>
  );
}

function Showcase() {
  return (
    <main className="transcript-showcase" data-testid="transcript-showcase">
      <h1>Transcript items</h1>
      <TranscriptItem
        icon={<Terminal />}
        meta="80 lines · 12s"
        openKey="exec"
        output={
          <TranscriptClamp lineCount={80}>
            <pre>{execLines}</pre>
          </TranscriptClamp>
        }
        quickAction={
          <Button size="sm" variant="ghost">
            Stop
          </Button>
        }
        status="running"
        target={<code>cargo test --workspace</code>}
        verb="Run"
      />
      <TranscriptItem
        icon={<FileText />}
        meta="120 lines"
        output={<pre>fn main() {"{}"}</pre>}
        target="src/main.rs"
        verb="Read"
      />
      <TranscriptItem
        icon={<Search />}
        meta="14 matches in 6 files"
        output={<pre>src/a.rs:12 limit</pre>}
        target="“limit”"
        verb="Search"
      />
      <TranscriptItem
        icon={<FilePen />}
        meta="+1 −1"
        output={<Diff />}
        target="src/config.rs"
        verb="Edit"
      />
      <TranscriptItem
        actions={
          <>
            <Button size="sm" variant="soft">
              Open page
            </Button>
            <Button size="sm" variant="ghost">
              Retry
            </Button>
          </>
        }
        artifacts={<span>screenshot-1.png · trace.zip</span>}
        error="TypeError: Cannot read properties of undefined (reading 'x')"
        icon={<Globe />}
        meta="1.2s"
        raw={[
          { label: "Request", value: { url: "https://example.com", wait: 1 } },
          { label: "Response", value: '{"status":200,"title":"Example"}' },
        ]}
        status="done"
        statusDetail="Errors from the page console"
        statusLabel="2 console errors"
        summary={[
          { label: "URL", value: "https://example.com" },
          { label: "Status", value: "200" },
          { label: "Title", value: "Example Domain" },
          { label: "Duration", value: "1.2s" },
        ]}
        target="example.com"
        verb="Browse"
      />
      <TranscriptItem
        icon={<Plug />}
        input={<pre>{'{ "query": "open issues", "limit": 5 }'}</pre>}
        meta="github"
        output="5 issues returned"
        target="search_issues"
        verb="MCP"
      />
      <TranscriptItem
        icon={<Send />}
        meta="2 lines"
        output="Please rebase on main before merging."
        separator="arrow"
        target="worker-1"
        verb="Message"
      />
      <TranscriptItem
        error="Recipient is no longer running"
        icon={<Send />}
        separator="arrow"
        status="failed"
        target="worker-2"
        verb="Message"
      />
      <TranscriptItem
        icon={<Bot />}
        meta="4m · 31 steps"
        output="Implemented the parser and added tests."
        target="Implement parser"
        verb="Subagent"
      />
      <TranscriptItem
        icon={<Bot />}
        meta="step 7"
        output="Reading project files…"
        status="running"
        target="Review the diff"
        verb="Subagent"
      />
      <TranscriptItem
        icon={<Users />}
        meta="3 running · 1 queued"
        status="waiting"
        summary={[
          { label: "Running", value: "3" },
          { label: "Queued", value: "1" },
        ]}
        target="Agents"
        verb="Status"
      />
      <TranscriptItem
        icon={<ListChecks />}
        meta="4 of 7 done"
        output="Step 5: wire adapter"
        target="Plan"
        verb="Tasks"
      />
      <TranscriptItem
        icon={<ShieldCheck />}
        meta="shell"
        target={<code>rm -rf target/</code>}
        verb="Approved"
      />
      <TranscriptItem
        actions={
          <>
            <Button size="sm" variant="primary">
              Answer
            </Button>
            <Button size="sm" variant="ghost">
              Skip
            </Button>
          </>
        }
        defaultOpen
        icon={<MessageCircleQuestion />}
        input="Which database should the migration target?"
        meta="1 question"
        status="needs_you"
        target="Which database?"
        verb="Question"
      />
      <TranscriptItem
        error="Upstream returned 502 Bad Gateway after 3 retries."
        icon={<TriangleAlert />}
        raw={[{ label: "Response", value: { status: 502, retries: 3 } }]}
        status="failed"
        target="Request failed"
        verb="Error"
      />
      <LegacyWrapperCard />
      <KitToolCard defaultOpen={false} icon={FileText} title="Kit card title">
        <pre>kit body</pre>
      </KitToolCard>
      <TranscriptItem
        icon={<Ban />}
        status="cancelled"
        target="Background build"
        verb="Run"
      />
    </main>
  );
}

const store = setUpStore();
const root = document.getElementById("root");
if (root) {
  createRoot(root).render(
    <React.StrictMode>
      <Provider store={store}>
        <Theme>
          <Showcase />
        </Theme>
      </Provider>
    </React.StrictMode>,
  );
}
