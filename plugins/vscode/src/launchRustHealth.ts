export const workerHealthProbeIntervalMs = 1000;
export const workerHealthFailureThreshold = 3;
export const workerHealthInitialBackoffMs = 1000;
export const workerHealthMaxBackoffMs = 30000;

export type WorkerHealthScheduler<Timer = ReturnType<typeof setTimeout>> = {
    setTimeout(callback: () => void, delayMs: number): Timer;
    clearTimeout(timer: Timer): void;
};

export type WorkerHealthProbeOptions<Timer = ReturnType<typeof setTimeout>> = {
    probe: () => Promise<boolean>;
    recover: () => void;
    scheduler?: WorkerHealthScheduler<Timer>;
    probeIntervalMs?: number;
    failureThreshold?: number;
    initialBackoffMs?: number;
    maxBackoffMs?: number;
};

const defaultScheduler: WorkerHealthScheduler = {
    setTimeout: (callback, delayMs) => setTimeout(callback, delayMs),
    clearTimeout: timer => clearTimeout(timer),
};

export class WorkerHealthProbe<Timer = ReturnType<typeof setTimeout>> {
    private readonly probe: () => Promise<boolean>;
    private readonly recover: () => void;
    private readonly scheduler: WorkerHealthScheduler<Timer>;
    private readonly probeIntervalMs: number;
    private readonly failureThreshold: number;
    private readonly initialBackoffMs: number;
    private readonly maxBackoffMs: number;
    private timer: Timer | undefined = undefined;
    private running = false;
    private disposed = false;
    private inFlight = false;
    private consecutiveFailures = 0;
    private backoffMs: number;
    private epoch = 0;

    constructor(options: WorkerHealthProbeOptions<Timer>) {
        this.probe = options.probe;
        this.recover = options.recover;
        this.scheduler = options.scheduler ?? (defaultScheduler as unknown as WorkerHealthScheduler<Timer>);
        this.probeIntervalMs = options.probeIntervalMs ?? workerHealthProbeIntervalMs;
        this.failureThreshold = options.failureThreshold ?? workerHealthFailureThreshold;
        this.initialBackoffMs = options.initialBackoffMs ?? workerHealthInitialBackoffMs;
        this.maxBackoffMs = options.maxBackoffMs ?? workerHealthMaxBackoffMs;
        this.backoffMs = this.initialBackoffMs;
    }

    public start(): void {
        if (this.disposed || this.running) {
            return;
        }
        this.running = true;
        this.reset();
        this.schedule(this.probeIntervalMs);
    }

    public stop(): void {
        this.running = false;
        this.inFlight = false;
        this.epoch++;
        this.clearTimer();
        this.reset();
    }

    public dispose(): void {
        this.disposed = true;
        this.stop();
    }

    public reset(): void {
        this.consecutiveFailures = 0;
        this.backoffMs = this.initialBackoffMs;
    }

    private schedule(delayMs: number): void {
        if (!this.running || this.disposed || this.timer) {
            return;
        }
        this.timer = this.scheduler.setTimeout(() => {
            this.timer = undefined;
            void this.tick();
        }, delayMs);
    }

    private async tick(): Promise<void> {
        if (!this.running || this.disposed || this.inFlight) {
            return;
        }
        const tickEpoch = this.epoch;
        this.inFlight = true;
        let healthy = false;
        try {
            healthy = await this.probe();
        } catch {
            healthy = false;
        } finally {
            if (this.epoch === tickEpoch) {
                this.inFlight = false;
            }
        }
        if (!this.running || this.disposed || this.epoch !== tickEpoch) {
            return;
        }
        if (healthy) {
            this.reset();
            this.schedule(this.probeIntervalMs);
            return;
        }
        this.consecutiveFailures++;
        if (this.consecutiveFailures < this.failureThreshold) {
            this.schedule(this.probeIntervalMs);
            return;
        }
        this.consecutiveFailures = 0;
        this.recover();
        const delayMs = this.backoffMs;
        this.backoffMs = Math.min(this.maxBackoffMs, this.backoffMs * 2);
        this.schedule(delayMs);
    }

    private clearTimer(): void {
        if (!this.timer) {
            return;
        }
        this.scheduler.clearTimeout(this.timer);
        this.timer = undefined;
    }
}
