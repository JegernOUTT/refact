import { EditorView } from "@codemirror/view";
import {
  createContext,
  useContext,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type ReactNode,
  type Dispatch,
  type SetStateAction,
} from "react";
import { Markdown } from "../../components/Markdown";
import { ArtifactBlock } from "../../components/Markdown/ArtifactBlock";
import {
  ArtifactPolicyContext,
  ArtifactPolicyProvider,
} from "../../components/Markdown/ArtifactPolicyContext";
import {
  ArrowUpRight,
  Code2,
  Info,
  MessageCircle,
  PanelTop,
} from "lucide-react";
import { Button, FieldTextarea, ToolCard } from "../../components/ui";
import {
  useAddReviewCommentMutation,
  useResolveAnchorMutation,
  reviewErrorMessage,
  type Anchor,
  type DocumentTarget,
  type Review,
  type ReviewCommentRequest,
  type RichBlock,
} from "../../services/refact/review";
import { ReviewFileDiff } from "./ReviewFileDiff";
import { useInlineDiscussions } from "./diff/useInlineDiscussions";
import {
  diffPanes,
  findCandidate,
  newRequestKey,
  objectAnchor,
  type PaneId,
} from "./model";
import { revealReview } from "./reviewNavigation";
import { setHighlightsEffect, type LineRange } from "./diff/reviewHighlights";
import { useReviewReadOnly } from "./readOnly";
import { PublishChoice, usePublishToPr } from "./ReviewRemote";
import { humanizeIdentifier } from "../../utils/displayNames";
import styles from "./ReviewDocument.module.css";

export const RichDiscussionContext = createContext<
  ((threadId: string) => ReactNode) | null
>(null);
export const RichAncestorThreads = createContext<ReadonlySet<string>>(
  new Set(),
);
export const RichFileNavigationContext = createContext<
  (candidateId: string, path: string) => void
>(() => undefined);
export const RichNavigationContext = createContext<(anchor: Anchor) => void>(
  () => undefined,
);

function targetKey(target: DocumentTarget) {
  return [
    target.candidate_id,
    target.step_id,
    target.version,
    target.block_id ?? "",
    target.block_revision ?? "",
  ];
}

type Draft = { body: string; request?: ReviewCommentRequest; error?: string };
export const ReviewComposerContext = createContext<{
  activeKey: string | null;
  setActiveKey: Dispatch<SetStateAction<string | null>>;
} | null>(null);
const DraftContext = createContext<{
  drafts: Map<string, Draft>;
  update: (key: string, draft: Draft) => void;
} | null>(null);
export function ReviewDocumentState({ children }: { children: ReactNode }) {
  const [drafts, setDrafts] = useState(new Map<string, Draft>());
  const [activeKey, setActiveKey] = useState<string | null>(null);
  return (
    <ReviewComposerContext.Provider value={{ activeKey, setActiveKey }}>
      <DraftContext.Provider
        value={{
          drafts,
          update: (key, draft) =>
            setDrafts((current) => new Map(current).set(key, draft)),
        }}
      >
        {children}
      </DraftContext.Provider>
    </ReviewComposerContext.Provider>
  );
}

function useReviewComposerOwnership() {
  const composer = useContext(ReviewComposerContext);
  const ownerId = useId();
  const [localOpen, setLocalOpen] = useState(false);
  const setActiveKey = composer?.setActiveKey;
  useEffect(
    () => () => {
      setActiveKey?.((current) => (current === ownerId ? null : current));
    },
    [ownerId, setActiveKey],
  );
  return {
    ownerId,
    open: composer ? composer.activeKey === ownerId : localOpen,
    setOpen: (value: boolean) => {
      if (setActiveKey)
        setActiveKey((current) =>
          value ? ownerId : current === ownerId ? null : current,
        );
      else setLocalOpen(value);
    },
  };
}

export function BlockQuestion({
  review,
  target,
  initiallyOpen = false,
}: {
  initiallyOpen?: boolean;
  review: Review;
  target: DocumentTarget;
}) {
  const shared = useContext(DraftContext);
  const renderThread = useContext(RichDiscussionContext);
  const { ownerId: composerId, open, setOpen } = useReviewComposerOwnership();
  useEffect(() => {
    if (initiallyOpen) setOpen(true);
    // Ownership is claimed only on the initial explicit host action.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [initiallyOpen]);
  const trigger = useRef<HTMLButtonElement>(null);
  const [local, setLocal] = useState<Draft>({ body: "" });
  const key = `${review.id}:document:${JSON.stringify(targetKey(target))}`;
  const draft = shared?.drafts.get(key) ?? local;
  const update = (value: Draft) =>
    shared ? shared.update(key, value) : setLocal(value);
  const [send, state] = useAddReviewCommentMutation();
  const readOnly = useReviewReadOnly();
  const submitting = useRef(false);
  const submit = async () => {
    if (readOnly || submitting.current || state.isLoading || !draft.body.trim())
      return;
    submitting.current = true;
    const request = draft.request ?? {
      reviewId: review.id,
      expected_revision: review.revision,
      request_key: newRequestKey(),
      target,
      body: draft.body.trim(),
      blocks: [],
    };
    update({ ...draft, request });
    const result = await send(request).finally(() => {
      submitting.current = false;
    });
    if ("data" in result) update({ body: "" });
    else {
      const status =
        typeof result.error === "object" && "status" in result.error
          ? result.error.status
          : null;
      const definite =
        typeof status === "number" && status >= 400 && status < 500;
      update({
        ...draft,
        request: definite ? undefined : request,
        error: reviewErrorMessage(result.error),
      });
    }
  };
  return (
    <div className={styles.question}>
      {review.threads
        .filter(
          (thread) =>
            thread.target &&
            JSON.stringify(targetKey(thread.target)) ===
              JSON.stringify(targetKey(target)),
        )
        .map((thread) => (
          <div key={thread.id} data-document-thread-id={thread.id}>
            {renderThread?.(thread.id)}
          </div>
        ))}
      <div className={styles.blockActions} data-active={open || undefined}>
        <Button
          ref={trigger}
          size="sm"
          variant="ghost"
          leftIcon={MessageCircle}
          aria-label="Ask about this block"
          aria-expanded={open}
          aria-controls={composerId}
          onClick={() => setOpen(!open)}
        >
          Ask{draft.error ? " · Retry needed" : draft.body ? " · Draft" : ""}
        </Button>
      </div>
      <div id={composerId} hidden={!open}>
        <form
          className={styles.composer}
          onSubmit={(event) => {
            event.preventDefault();
            void submit();
          }}
        >
          <FieldTextarea
            aria-label="Question about this block"
            placeholder="Ask a question about this explanation…"
            value={draft.body}
            readOnly={!!draft.request}
            disabled={readOnly}
            onChange={(body) => update({ body })}
          />
          {draft.error && (
            <p role="alert" className={styles.error}>
              {draft.error}
              {draft.request
                ? " · Outcome unknown; retry sends the identical request."
                : " · Draft kept."}
            </p>
          )}
          <Button
            type="submit"
            variant="primary"
            size="sm"
            disabled={readOnly || !draft.body.trim()}
            loading={state.isLoading}
          >
            {draft.request ? "Retry question" : "Ask"}
          </Button>
          {!draft.request && !draft.error && (
            <Button
              size="sm"
              variant="ghost"
              onClick={() => {
                setOpen(false);
                trigger.current?.focus();
              }}
            >
              Cancel
            </Button>
          )}
        </form>
      </div>
    </div>
  );
}

function NativeBlock({ review, block }: { review: Review; block: RichBlock }) {
  const content = block.content;
  const anchor =
    content.kind === "code"
      ? content.anchor
      : content.kind === "diff"
        ? content.anchors?.at(0)
        : undefined;
  const candidateId =
    content.kind === "diff"
      ? content.candidate_id
      : anchor?.position.candidate_id ?? null;
  const path =
    content.kind === "diff" ? content.path : anchor?.position.path ?? "";
  const [diffMode, setDiffMode] = useState<"unified" | "side-by-side">(
    "side-by-side",
  );
  const candidate = findCandidate(review, candidateId);
  const nativeRoot = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const root = nativeRoot.current;
    if (!root || !anchor) return;
    let placed = false;
    let disposed = false;
    const place = () => {
      if (placed) return;
      const editors = root.querySelectorAll<HTMLElement>(".cm-editor");
      const index =
        content.kind === "code" || anchor.position.side === "head"
          ? editors.length - 1
          : 0;
      const editor = Array.from(editors).at(index);
      const view = editor ? EditorView.findFromDOM(editor) : null;
      if (
        !view ||
        anchor.position.start_line < 1 ||
        anchor.position.start_line > view.state.doc.lines
      )
        return;
      placed = true;
      if (anchor.status === "current")
        view.dispatch({
          effects: setHighlightsEffect.of([
            {
              id: `native-range:${block.id}`,
              kind: "tour",
              startLine: anchor.position.start_line,
              endLine: anchor.position.end_line,
              active: true,
            },
          ]),
        });
      view.requestMeasure({
        read: (current) =>
          current.lineBlockAt(
            current.state.doc.line(anchor.position.start_line).from,
          ).top,
        write: (top, current) => {
          if (!disposed) current.scrollDOM.scrollTop = top;
        },
      });
    };
    place();
    const observer = new MutationObserver(place);
    observer.observe(root, { childList: true, subtree: true });
    return () => {
      disposed = true;
      observer.disconnect();
    };
  }, [anchor, block.id, content.kind, diffMode]);
  const panes = useMemo(() => {
    if (!candidate) return null;
    if (content.kind === "code" && anchor) {
      const source = { candidateId: candidate.id, side: anchor.position.side };
      return { left: source, right: source };
    }
    return diffPanes(candidate, null, "full");
  }, [candidate, content.kind, anchor]);
  const [draftAnchors, setDraftAnchors] = useState<Anchor[]>([]);
  const [selection, setSelection] = useState<{
    pane: PaneId;
    range: LineRange;
  } | null>(null);
  const [resolve] = useResolveAnchorMutation();
  const [error, setError] = useState<string | null>(null);
  const [counts, setCounts] = useState<{ left: number; right: number }>();
  const ancestors = useContext(RichAncestorThreads);
  const readOnly = useReviewReadOnly();
  const navigate = useContext(RichNavigationContext);
  const navigateFile = useContext(RichFileNavigationContext);
  const discussions = useInlineDiscussions({
    review: {
      ...review,
      threads: review.threads.filter((thread) => !ancestors.has(thread.id)),
    },
    panes,
    path,
    draftAnchors,
    lineCounts: counts,
    onCloseDraft: (a) =>
      setDraftAnchors((current) => current.filter((item) => item !== a)),
    activeObjectId: null,
    onFocusObject: (objectId) => {
      const object = objectAnchor(review, objectId);
      if (object) navigate(object);
      else revealReview({ reviewId: review.id, threadId: objectId });
    },
    focusAllowed: () => false,
  });
  const comment = async (pane: PaneId, range: LineRange) => {
    if (!panes || readOnly) return;
    const source = panes[pane];
    const result = await resolve({
      reviewId: review.id,
      candidate_id: source.candidateId,
      path,
      side: source.side,
      start_line: range.startLine,
      end_line: range.endLine,
    });
    if (result.data) {
      const fresh = result.data;
      setDraftAnchors((current) =>
        current.some(
          (a) => JSON.stringify(a.position) === JSON.stringify(fresh.position),
        )
          ? current
          : [...current, fresh],
      );
    } else setError(reviewErrorMessage(result.error));
  };
  if (!candidate || !panes)
    return <p role="status">Referenced candidate is unavailable.</p>;
  return (
    <>
      <div ref={nativeRoot} className={styles.native}>
        <div className={styles.nativeHeader}>
          <div className={styles.nativeIdentity}>
            <Code2 aria-hidden="true" />
            <span className={styles.nativePath}>{path}</span>
            <span className={styles.nativeContext}>
              {candidate.id.slice(0, 8)}
              {anchor
                ? ` · ${anchor.position.side} L${anchor.position.start_line}–${anchor.position.end_line}`
                : ""}
            </span>
          </div>
          <Button
            size="sm"
            variant="soft"
            leftIcon={ArrowUpRight}
            onClick={() =>
              anchor ? navigate(anchor) : navigateFile(candidate.id, path)
            }
          >
            Open full Review
          </Button>
          {content.kind === "diff" && (
            <Button
              size="sm"
              variant="ghost"
              onClick={() =>
                setDiffMode((current) =>
                  current === "unified" ? "side-by-side" : "unified",
                )
              }
            >
              {diffMode === "unified" ? "Split diff" : "Unified diff"}
            </Button>
          )}
          {selection && (
            <Button
              size="sm"
              disabled={readOnly}
              onClick={() => void comment(selection.pane, selection.range)}
            >
              Comment on selection
            </Button>
          )}
        </div>
        <ToolCard
          title="Reference metadata"
          icon={Info}
          density="compact"
          defaultOpen={false}
          className={styles.disclosure}
          animate={false}
        >
          <dl className={styles.metadata}>
            <dt>Candidate</dt>
            <dd>{candidate.id}</dd>
            <dt>Base</dt>
            <dd>{candidate.base_oid}</dd>
            <dt>Head</dt>
            <dd>{candidate.head_oid}</dd>
            <dt>Content digest</dt>
            <dd>{candidate.content_digest}</dd>
            <dt>Path</dt>
            <dd>{path}</dd>
            {anchor && (
              <>
                <dt>Source range</dt>
                <dd>
                  {anchor.position.side} L{anchor.position.start_line}–
                  {anchor.position.end_line}
                </dd>
                <dt>Anchor status</dt>
                <dd>{anchor.status}</dd>
              </>
            )}
          </dl>
        </ToolCard>
        {error && (
          <p role="alert" className={styles.error}>
            {error}
          </p>
        )}
        <div className={styles.editor}>
          <ReviewFileDiff
            review={review}
            panes={panes}
            path={path}
            diffMode={content.kind === "code" ? "unified" : diffMode}
            activeObjectId={null}
            reveal={null}
            autoRevealFirstChange={false}
            inlineHosts={discussions.inlineHosts}
            onLineCounts={(value) =>
              setCounts((current) =>
                current?.left === value.left && current.right === value.right
                  ? current
                  : value,
              )
            }
            onSelection={(pane, range) =>
              setSelection(range ? { pane, range } : null)
            }
            onCommentLine={(pane, line) =>
              void comment(pane, { startLine: line, endLine: line })
            }
          />
        </div>
        {discussions.portals}
      </div>
    </>
  );
}

export function ReviewSourceContext({
  review,
  anchor,
  onClose,
}: {
  review: Review;
  anchor: Anchor;
  onClose?: () => void;
}) {
  if (anchor.status !== "current")
    return <p role="status">Stalled range · inspect the file instead.</p>;
  return (
    <section aria-label="Exact source context" className={styles.sourceContext}>
      {onClose && (
        <Button size="sm" variant="ghost" onClick={onClose}>
          Close source context
        </Button>
      )}
      <NativeBlock
        review={review}
        block={{
          id: "source-context",
          revision: 1,
          content: { kind: "code", anchor },
        }}
      />
    </section>
  );
}

export function RichBlockContent({
  review,
  block,
  bindingKey,
}: {
  review: Review;
  block: RichBlock;
  bindingKey: string;
}) {
  const navigate = useContext(RichNavigationContext);
  const navigateFile = useContext(RichFileNavigationContext);
  const content = block.content;
  const [contextAnchor, setContextAnchor] = useState<Anchor | null>(null);
  if (content.kind === "markdown") return <Markdown>{content.body}</Markdown>;
  if (content.kind === "html")
    return (
      <ArtifactPolicyProvider
        value={{
          mode: "review-seamless",
          bindingKey,
          title: "Tour visual",
          revealAnchorCount: content.anchors?.length ?? 0,
          onRevealAnchor: (index) => {
            const anchor = content.anchors?.at(index);
            if (anchor && anchor.status === "current") navigate(anchor);
          },
        }}
      >
        <ArtifactBlock
          code={content.html}
          staticFallback={content.static_fallback}
        />
      </ArtifactPolicyProvider>
    );
  if (content.kind === "code" || content.kind === "diff")
    return <NativeBlock review={review} block={block} />;
  const evidence = [...new Set(content.evidence_ids)].flatMap((id) => {
    const item = review.evidence.find((e) => e.id === id);
    return item ? [item] : [];
  });
  const sources = evidence.filter(
    (item) => content.kind !== "sources" || item.source.kind !== "executed",
  );
  return (
    <section
      aria-label={
        content.kind === "sources" ? `${content.relation} sources` : "Evidence"
      }
    >
      <h4>
        {content.kind === "sources"
          ? `${
              content.relation === "consulted" ? "Consulted" : "Supporting"
            } sources`
          : "Evidence"}
      </h4>
      <ul className={styles.evidenceList}>
        {sources.map((item) => (
          <li
            key={item.id}
            className={styles.evidenceCard}
            data-evidence-id={item.id}
          >
            <div className={styles.evidenceMeta}>
              <span>
                {content.kind === "sources"
                  ? content.relation === "consulted"
                    ? "Consulted"
                    : "Supporting"
                  : "Evidence"}{" "}
                ·{" "}
                {item.source.kind === "source"
                  ? "Source"
                  : item.source.kind === "external"
                    ? "External reference"
                    : "Executed"}
              </span>
              <span title={item.candidate_id}>
                Candidate {item.candidate_id.slice(0, 8)}
              </span>
              <span>
                {item.source.kind === "source"
                  ? item.source.anchor
                    ? item.source.anchor.status === "current"
                      ? "Current · exact range"
                      : "Stalled · file only"
                    : "File only · no exact range"
                  : humanizeIdentifier(item.applicability)}
              </span>
            </div>
            {item.source.kind === "source" ? (
              <Button
                size="sm"
                variant="plain"
                leftIcon={Code2}
                disabled={
                  !review.candidates.some(
                    (candidate) =>
                      candidate.id === item.candidate_id &&
                      candidate.files.some(
                        (file) =>
                          item.source.kind === "source" &&
                          file.path === item.source.path,
                      ),
                  )
                }
                onClick={() => {
                  if (item.source.kind !== "source") return;
                  if (item.source.anchor?.status === "current")
                    navigate(item.source.anchor);
                  else navigateFile(item.candidate_id, item.source.path);
                }}
              >
                {item.source.path}
                {item.source.anchor &&
                  ` · ${item.source.anchor.position.side} L${item.source.anchor.position.start_line}–${item.source.anchor.position.end_line}`}
              </Button>
            ) : item.source.kind === "external" ? (
              /^https?:\/\//i.test(item.source.url) ? (
                <a href={item.source.url} target="_blank" rel="noreferrer">
                  {item.source.url}
                </a>
              ) : (
                item.source.url
              )
            ) : (
              <span>
                Executed: {item.source.command.join(" ")} · exit{" "}
                {item.source.exit_code ?? "unknown"}
              </span>
            )}
            {item.source.kind === "source" &&
              item.source.anchor?.status === "current" && (
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => {
                    if (item.source.kind === "source")
                      setContextAnchor(item.source.anchor);
                  }}
                >
                  Inspect exact range
                </Button>
              )}
            <p className={styles.evidenceSummary}>{item.summary}</p>
            <div className={styles.evidenceMeta}>
              <span>Evidence {item.id}</span>
              {item.material_id && <span>Material {item.material_id}</span>}
              {item.run_id && <span>Run {item.run_id}</span>}
              {item.source.kind === "source" &&
                item.source.anchor?.status === "stalled" && (
                  <span>
                    {item.source.anchor.stalled_reason ??
                      "Exact range needs revalidation"}
                  </span>
                )}
            </div>
          </li>
        ))}
      </ul>
      {contextAnchor && (
        <ReviewSourceContext
          review={review}
          anchor={contextAnchor}
          onClose={() => setContextAnchor(null)}
        />
      )}
    </section>
  );
}

function LazyBlock({ children }: { children: ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);
  const [visited, setVisited] = useState(
    () => typeof IntersectionObserver === "undefined",
  );
  useEffect(() => {
    if (visited || !ref.current || typeof IntersectionObserver === "undefined")
      return;
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) setVisited(true);
      },
      { rootMargin: "300px" },
    );
    observer.observe(ref.current);
    return () => observer.disconnect();
  }, [visited]);
  return (
    <div ref={ref}>
      {visited ? (
        children
      ) : (
        <Button
          className={styles.loadBlock}
          variant="soft"
          size="sm"
          leftIcon={PanelTop}
          onClick={() => setVisited(true)}
        >
          Load interactive block
        </Button>
      )}
    </div>
  );
}

export function ReviewRichBlocks({
  review,
  blocks,
  body,
  target,
  bindingKey,
}: {
  review: Review;
  blocks?: RichBlock[];
  body: string;
  target?: DocumentTarget;
  bindingKey: string;
}) {
  const inheritedPolicy = useContext(ArtifactPolicyContext);
  const ancestors = useContext(RichAncestorThreads);
  const renderThread = useContext(RichDiscussionContext);
  const documentOwned = new Set(ancestors);
  if (target && renderThread)
    for (const thread of review.threads) {
      const threadTarget = thread.target;
      if (
        threadTarget &&
        blocks?.some(
          (block) =>
            JSON.stringify(targetKey(threadTarget)) ===
            JSON.stringify(
              targetKey({
                ...target,
                block_id: block.id,
                block_revision: block.revision,
              }),
            ),
        )
      )
        documentOwned.add(thread.id);
    }
  if (!blocks?.length)
    return (
      <ArtifactPolicyProvider
        value={{ ...inheritedPolicy, mode: "review-seamless", bindingKey }}
      >
        <Markdown>{body}</Markdown>
      </ArtifactPolicyProvider>
    );
  return (
    <RichAncestorThreads.Provider value={documentOwned}>
      {body.trim() &&
        !blocks.some(
          (block) =>
            block.content.kind === "markdown" && block.content.body === body,
        ) && (
          <ArtifactPolicyProvider
            value={{
              ...inheritedPolicy,
              mode: "review-seamless",
              bindingKey: `${bindingKey}:body`,
            }}
          >
            <Markdown>{body}</Markdown>
          </ArtifactPolicyProvider>
        )}
      {blocks.map((block) => (
        <section
          key={block.id}
          data-block-id={block.id}
          className={styles.block}
        >
          {block.content.kind === "markdown" ? (
            <RichBlockContent
              review={review}
              block={block}
              bindingKey={`${bindingKey}:${block.id}`}
            />
          ) : (
            <LazyBlock>
              <RichBlockContent
                review={review}
                block={block}
                bindingKey={`${bindingKey}:${block.id}`}
              />
            </LazyBlock>
          )}
          {target && (
            <BlockQuestion
              review={review}
              target={{
                ...target,
                block_id: block.id,
                block_revision: block.revision,
              }}
            />
          )}
        </section>
      ))}
    </RichAncestorThreads.Provider>
  );
}

export function SharedReviewComposer({
  reviewId,
  revision,
  anchor,
  threadId,
  annotationId,
  label,
  ownerId,
  focusOnOpen = false,
  onClose,
  open = true,
  publishable = true,
}: {
  reviewId: string;
  revision: number;
  anchor?: Anchor;
  threadId?: string;
  annotationId?: string;
  label?: string;
  ownerId?: string;
  focusOnOpen?: boolean;
  onClose?: () => void;
  open?: boolean;
  /** Code comments can also go to the pull request; tour questions never do. */
  publishable?: boolean;
}) {
  const textarea = useRef<HTMLTextAreaElement>(null);
  useEffect(() => {
    if (open && focusOnOpen) textarea.current?.focus();
  }, [open, focusOnOpen]);
  const shared = useContext(DraftContext);
  const key = JSON.stringify([
    reviewId,
    threadId
      ? ["thread", threadId]
      : ["anchor", anchor?.position, annotationId],
  ]);
  const instanceId = useId();
  const activeOwner = ownerId ?? instanceId;
  const composer = useContext(ReviewComposerContext);
  const setActiveKey = composer?.setActiveKey;
  useEffect(() => {
    if (!ownerId && open) setActiveKey?.(activeOwner);
  }, [activeOwner, ownerId, open, setActiveKey]);
  useEffect(
    () => () => {
      setActiveKey?.((current) => (current === activeOwner ? null : current));
    },
    [activeOwner, setActiveKey],
  );
  const submitting = useRef(false);
  const [local, setLocal] = useState<Draft>({ body: "" });
  const draft = shared?.drafts.get(key) ?? local;
  const update = (value: Draft) =>
    shared ? shared.update(key, value) : setLocal(value);
  const [send, state] = useAddReviewCommentMutation();
  const readOnly = useReviewReadOnly();
  const publish = usePublishToPr() && publishable;
  const submit = async () => {
    if (readOnly || submitting.current || state.isLoading || !draft.body.trim())
      return;
    submitting.current = true;
    const request = draft.request ?? {
      reviewId,
      expected_revision: revision,
      request_key: newRequestKey(),
      body: draft.body.trim(),
      anchor,
      thread_id: threadId,
      ...(annotationId ? { annotation_id: annotationId } : {}),
      ...(annotationId ? {} : { blocks: [] }),
      ...(publish ? { publish: true } : {}),
    };
    update({ ...draft, request });
    const result = await send(request).finally(() => {
      submitting.current = false;
    });
    if (result.data) {
      update({ body: "" });
      onClose?.();
    } else {
      const status =
        typeof result.error === "object" && "status" in result.error
          ? result.error.status
          : null;
      update({
        ...draft,
        request:
          typeof status === "number" && status >= 400 && status < 500
            ? undefined
            : request,
        error: reviewErrorMessage(result.error),
      });
    }
  };
  return (
    <form
      hidden={!open || (!!composer && composer.activeKey !== activeOwner)}
      className={styles.composer}
      onSubmit={(event) => {
        event.preventDefault();
        void submit();
      }}
    >
      <label>
        {" "}
        {label ?? (threadId ? "Reply" : "Comment on code")}
        <FieldTextarea
          ref={textarea}
          onKeyDown={(event) => {
            if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) {
              event.preventDefault();
              void submit();
            } else if (event.key === "Escape" && !draft.request && onClose) {
              event.preventDefault();
              onClose();
            }
          }}
          aria-label={
            label ??
            (threadId
              ? "Reply"
              : `New comment on ${anchor?.position.path ?? "code"}`)
          }
          value={draft.body}
          readOnly={!!draft.request}
          disabled={readOnly}
          onChange={(body) => update({ body })}
        />
      </label>
      {draft.error && (
        <p role="alert" className={styles.error}>
          {draft.error}
          {draft.request
            ? " · Not confirmed; retry sends the identical request."
            : " · Draft kept."}
        </p>
      )}
      <Button
        size="sm"
        variant="primary"
        type="submit"
        disabled={readOnly || !draft.body.trim()}
        loading={state.isLoading}
      >
        {draft.request ? "Retry" : "Comment"}
      </Button>
      {!draft.request && onClose && (
        <Button size="sm" variant="ghost" onClick={onClose}>
          Cancel
        </Button>
      )}
      {publishable && !draft.request && <PublishChoice />}
    </form>
  );
}
