import {
    applyDecisionResolution,
    emptyDecisionAnnouncementState,
    isValidAskQuestionsPayload,
    outstandingDecisionCount,
    scopeDecisionState,
    shouldAnnounceDecision,
    type DecisionScope,
} from "refact-chat-js/dist/events";

/** Host lifecycle adapter only. All decision semantics live in GUI's built contract. */
export class HostDecisionNotifications {
    private state = emptyDecisionAnnouncementState();
    private generation = 0;
    private scope: DecisionScope | undefined;
    private allowLegacyScope = true;
    private disposed = false;

    constructor(private readonly effects: {
        badge: (count: number) => void;
        notify: (message: string) => PromiseLike<string | undefined>;
        openChat: (chatId: string) => void;
    }) {}

    bind(scope: DecisionScope | undefined): number {
        if (this.disposed) { return this.generation; }
        if (JSON.stringify(scope) !== JSON.stringify(this.scope)) {
            if (scope && this.state.scope &&
                (scope.projectRoot !== this.state.scope.projectRoot ||
                    scope.endpointIdentity !== this.state.scope.endpointIdentity)) {
                // After a rebind an unscoped legacy event cannot identify its engine.
                this.allowLegacyScope = false;
            }
            this.generation++;
            this.scope = scope;
            // A temporary disconnect is not a resolution and must retain tombstones.
            if (scope) { this.state = scopeDecisionState(this.state, scope); }
        }
        this.effects.badge(this.scope ? outstandingDecisionCount(this.state) : 0);
        return this.generation;
    }

    /** Replacing a view invalidates delayed callbacks but not pending decisions. */
    reconnect(): number { return ++this.generation; }

    ask(payload: unknown, generation = this.generation): void {
        if (!this.accepts(payload, generation)) { return; }
        const outcome = shouldAnnounceDecision(this.state, payload, this.scope);
        this.state = outcome.next;
        this.effects.badge(outstandingDecisionCount(this.state));
        if (!outcome.announce || !isValidAskQuestionsPayload(payload)) { return; }
        const count = payload.questions.length;
        const text = count === 1 ? "1 question" : `${count} questions`;
        void this.effects.notify(`AI needs ${text} to continue`).then(selection => {
            if (selection === "Open Chat" && generation === this.generation) {
                // Opening/reading a chat is not a native decision resolution.
                this.effects.openChat(payload.chatId);
            }
        });
    }

    resolved(payload: unknown, generation = this.generation): void {
        if (!this.accepts(payload, generation)) { return; }
        this.state = applyDecisionResolution(this.state, payload).next;
        this.effects.badge(outstandingDecisionCount(this.state));
    }

    taskDone(payload: unknown): void {
        if (this.disposed || !payload || typeof payload !== "object") { return; }
        const task = payload as { chatId?: unknown; summary?: unknown };
        if (typeof task.chatId !== "string" || !task.chatId || typeof task.summary !== "string") { return; }
        const chatId = task.chatId;
        const generation = this.generation;
        void this.effects.notify(task.summary || "Task completed").then(selection => {
            if (selection === "Open Chat" && generation === this.generation) {
                this.effects.openChat(chatId);
            }
        });
        // Task announcements never modify outstanding decisions or tombstones.
    }

    dispose(): void {
        this.disposed = true;
        this.generation++;
        this.scope = undefined;
        this.state = emptyDecisionAnnouncementState();
        this.effects.badge(0);
    }

    private accepts(payload: unknown, generation: number): boolean {
        return !this.disposed && this.scope !== undefined && generation === this.generation &&
            (this.allowLegacyScope || (!!payload && typeof payload === "object" &&
                (payload as { scope?: unknown }).scope !== undefined));
    }
}
