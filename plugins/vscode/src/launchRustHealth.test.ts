import * as assert from "assert";
import {
    WorkerHealthProbe,
    type WorkerHealthScheduler,
} from "./launchRustHealth";

type FakeTimer = number;

type ScheduledTimer = {
    id: FakeTimer;
    callback: () => void;
    delayMs: number;
};

class FakeScheduler implements WorkerHealthScheduler<FakeTimer> {
    private nextId = 1;
    public scheduled: ScheduledTimer[] = [];
    public clears: FakeTimer[] = [];

    setTimeout(callback: () => void, delayMs: number): FakeTimer {
        const id = this.nextId++;
        this.scheduled.push({ id, callback, delayMs });
        return id;
    }

    clearTimeout(timer: FakeTimer): void {
        this.clears.push(timer);
        this.scheduled = this.scheduled.filter(entry => entry.id !== timer);
    }

    pendingDelay(): number | undefined {
        return this.scheduled[0]?.delayMs;
    }

    pendingCount(): number {
        return this.scheduled.length;
    }

    async fireNext(): Promise<number | undefined> {
        const next = this.scheduled.shift();
        if (!next) {
            return undefined;
        }
        next.callback();
        await flushMicrotasks();
        return next.delayMs;
    }
}

function flushMicrotasks(): Promise<void> {
    return new Promise(resolve => setImmediate(resolve));
}

function controllableProbe(results: boolean[], fallback = true): { probe: () => Promise<boolean>; calls: () => number } {
    const queue = [...results];
    let calls = 0;
    return {
        probe: async () => {
            calls++;
            return queue.length > 0 ? queue.shift() ?? fallback : fallback;
        },
        calls: () => calls,
    };
}

async function transientFailureDoesNotRecover() {
    const scheduler = new FakeScheduler();
    let recoverCalls = 0;
    const { probe, calls } = controllableProbe([false, true, true]);
    const health = new WorkerHealthProbe<FakeTimer>({
        probe,
        recover: () => { recoverCalls++; },
        scheduler,
        probeIntervalMs: 1000,
        failureThreshold: 3,
        initialBackoffMs: 1000,
        maxBackoffMs: 30000,
    });

    health.start();
    assert.strictEqual(scheduler.pendingDelay(), 1000);

    await scheduler.fireNext();
    assert.strictEqual(recoverCalls, 0);
    assert.strictEqual(scheduler.pendingDelay(), 1000);

    await scheduler.fireNext();
    assert.strictEqual(recoverCalls, 0);
    assert.strictEqual(scheduler.pendingDelay(), 1000);

    assert.strictEqual(calls(), 2);
    health.dispose();
}

async function sustainedFailureRecoversWithBackoff() {
    const scheduler = new FakeScheduler();
    let recoverCalls = 0;
    const { probe } = controllableProbe([], false);
    const health = new WorkerHealthProbe<FakeTimer>({
        probe,
        recover: () => { recoverCalls++; },
        scheduler,
        probeIntervalMs: 1000,
        failureThreshold: 3,
        initialBackoffMs: 1000,
        maxBackoffMs: 4000,
    });

    health.start();

    const backoffDelays: number[] = [];
    for (let recovery = 0; recovery < 4; recovery++) {
        await scheduler.fireNext();
        await scheduler.fireNext();
        const recoverCallsBefore = recoverCalls;
        await scheduler.fireNext();
        assert.strictEqual(recoverCalls, recoverCallsBefore + 1);
        backoffDelays.push(scheduler.pendingDelay() ?? -1);
    }

    assert.deepStrictEqual(backoffDelays, [1000, 2000, 4000, 4000]);
    assert.strictEqual(recoverCalls, 4);
    health.dispose();
}

async function successResetsConsecutiveFailures() {
    const scheduler = new FakeScheduler();
    let recoverCalls = 0;
    const { probe } = controllableProbe([false, false, true, false, false, true]);
    const health = new WorkerHealthProbe<FakeTimer>({
        probe,
        recover: () => { recoverCalls++; },
        scheduler,
        probeIntervalMs: 1000,
        failureThreshold: 3,
        initialBackoffMs: 1000,
        maxBackoffMs: 30000,
    });

    health.start();
    for (let i = 0; i < 6; i++) {
        await scheduler.fireNext();
    }

    assert.strictEqual(recoverCalls, 0);
    assert.strictEqual(scheduler.pendingDelay(), 1000);
    health.dispose();
}

async function disposeStopsProbeWithoutLeakingTimers() {
    const scheduler = new FakeScheduler();
    let recoverCalls = 0;
    const { probe } = controllableProbe([], false);
    const health = new WorkerHealthProbe<FakeTimer>({
        probe,
        recover: () => { recoverCalls++; },
        scheduler,
        probeIntervalMs: 1000,
        failureThreshold: 3,
    });

    health.start();
    assert.strictEqual(scheduler.pendingCount(), 1);
    const scheduledId = scheduler.scheduled[0]?.id;

    health.dispose();
    assert.strictEqual(scheduler.pendingCount(), 0);
    assert.strictEqual(scheduler.clears.includes(scheduledId ?? -1), true);

    health.start();
    assert.strictEqual(scheduler.pendingCount(), 0);
    assert.strictEqual(recoverCalls, 0);
}

async function stopDuringInFlightProbeDoesNotRecover() {
    const scheduler = new FakeScheduler();
    let recoverCalls = 0;
    let resolveProbe: ((value: boolean) => void) | undefined;
    const health = new WorkerHealthProbe<FakeTimer>({
        probe: () => new Promise<boolean>(resolve => { resolveProbe = resolve; }),
        recover: () => { recoverCalls++; },
        scheduler,
        probeIntervalMs: 1000,
        failureThreshold: 1,
    });

    health.start();
    const pending = scheduler.scheduled.shift();
    assert.notStrictEqual(pending, undefined);
    pending?.callback();
    await flushMicrotasks();

    health.stop();
    resolveProbe?.(false);
    await flushMicrotasks();

    assert.strictEqual(recoverCalls, 0);
    assert.strictEqual(scheduler.pendingCount(), 0);
    health.dispose();
}

export async function runLaunchRustHealthTests() {
    await transientFailureDoesNotRecover();
    await sustainedFailureRecoversWithBackoff();
    await successResetsConsecutiveFailures();
    await disposeStopsProbeWithoutLeakingTimers();
    await stopDuringInFlightProbeDoesNotRecover();
}

runLaunchRustHealthTests().catch(error => {
    console.error(error);
    process.exitCode = 1;
});
