const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const bundle = fs.readFileSync(path.join(__dirname, '../../src/generated/injected_bundle.js'), 'utf8');

// Execute the shipped bundle, including its getter-style bootstrap export, exactly
// as injected_source.rs does. No source extraction or alternate implementation.
function fixture(rectangle = () => ({ x: 1, y: 2, width: 30, height: 40 })) {
  let now = 0;
  let id = 0;
  let reads = 0;
  const timers = new Map();
  const frames = new Map();
  const callbacks = [];
  const forbidden = () => { throw new Error('page fake-clock API must not be used'); };
  const fakeGlobal = {
    setTimeout: forbidden, clearTimeout: forbidden,
    requestAnimationFrame: forbidden, cancelAnimationFrame: forbidden,
    performance: { now: forbidden },
  };
  const builtins = {
    Map, Set, WeakMap,
    performanceNow: () => now,
    setTimeout(callback, delay) {
      const handle = ++id;
      callbacks.push(callback);
      timers.set(handle, { callback, at: now + delay });
      return handle;
    },
    clearTimeout: handle => timers.delete(handle),
    requestAnimationFrame(callback) {
      const handle = ++id;
      callbacks.push(callback);
      frames.set(handle, callback);
      return handle;
    },
    cancelAnimationFrame: handle => frames.delete(handle),
  };
  const context = vm.createContext({ module: { exports: {} } });
  vm.runInContext(bundle, context, { filename: 'injected_bundle.js' });
  const injected = context.module.exports.bootstrapRefactInjected()(fakeGlobal, builtins);
  const element = {
    isConnected: true,
    getBoundingClientRect() { ++reads; return rectangle(reads); },
  };
  return {
    injected, element,
    get reads() { return reads; },
    // Model an already-queued frame that cancellation cannot retract, without
    // delivering the newly scheduled frame as well.
    queuedFrame() { return [...frames.values()][0]; },
    deliverQueued(callback, time) { now = time; callback(time); },
    advance(time) {
      for (;;) {
        const next = [...timers].filter(([, entry]) => entry.at <= time)
          .sort((a, b) => a[1].at - b[1].at || a[0] - b[0])[0];
        if (!next) break;
        now = next[1].at;
        timers.delete(next[0]);
        next[1].callback();
      }
      now = time;
    },
    frame(time) {
      now = time;
      const pending = [...frames.values()];
      frames.clear();
      for (const callback of pending) callback(time);
    },
    assertClean() {
      assert.equal(timers.size, 0, 'all native timers cancelled');
      assert.equal(frames.size, 0, 'all native frames cancelled');
      const before = reads;
      for (const callback of callbacks) callback(now);
      assert.equal(reads, before, 'stale callbacks cannot read geometry after settlement');
      assert.equal(timers.size + frames.size, 0, 'stale callbacks cannot reschedule');
    },
  };
}

const plain = value => JSON.parse(JSON.stringify(value));

test('static geometry succeeds with native rAF never delivered and page clock unusable', async () => {
  const f = fixture();
  const result = f.injected.elementState(f.element, 'stable', 100);
  f.advance(100);
  assert.deepEqual(plain(await result), { stable: true, matches: true });
  assert.equal(f.reads, 2);
  f.assertClean();
});

test('each exact geometry component still detects movement without native rAF', async () => {
  for (const field of ['x', 'y', 'width', 'height']) {
    const f = fixture(read => ({ x: 1, y: 2, width: 30, height: 40, [field]: read / 100 }));
    const result = f.injected.elementState(f.element, 'stable', 100);
    f.advance(100);
    assert.deepEqual(plain(await result), { stable: false, matches: false }, field);
    f.assertClean();
  }
});

test('native rAF remains primary and samples less than 15ms apart cannot settle (including time zero)', async () => {
  const f = fixture();
  const result = f.injected.elementState(f.element, 'stable', 100);
  f.frame(0);
  assert.equal(f.reads, 1);
  f.frame(1);
  f.frame(14);
  assert.equal(f.reads, 1, 'no second geometry sample before minimum separation');
  f.frame(15);
  assert.deepEqual(plain(await result), { stable: true, matches: true });
  f.assertClean();
});

test('queued cancelled frame before settlement cannot bypass spacing or movement comparison', async () => {
  for (const moving of [false, true]) {
    const f = fixture(read => ({ x: moving ? read : 1, y: 2, width: 30, height: 40 }));
    const result = f.injected.elementState(f.element, 'stable', 100);
    const queued = f.queuedFrame();
    f.advance(32); // Timer wins, cancels the queued old frame, and samples once.
    assert.equal(f.reads, 1);
    f.deliverQueued(queued, 33);
    f.deliverQueued(queued, 46);
    assert.equal(f.reads, 1, 'stale callback cannot sample within 15ms');
    f.deliverQueued(queued, 47);
    assert.deepEqual(plain(await result), { stable: !moving, matches: !moving });
    assert.equal(f.reads, 2, 'stale callback uses the same exact comparator');
    f.assertClean();
  }
});

test('detachment during starved-rAF polling still rejects and cleans up', async () => {
  const f = fixture();
  const result = f.injected.elementState(f.element, 'stable', 100);
  const rejection = assert.rejects(result, /Element is not attached to the DOM/);
  f.element.isConnected = false;
  f.advance(100);
  await rejection;
  f.assertClean();
});

test('deadline remains bounded when there is not enough time for two samples', async () => {
  const f = fixture();
  const result = f.injected.elementState(f.element, 'stable', 10);
  f.advance(10);
  assert.deepEqual(plain(await result), { stable: false, unstable: 'deadline', matches: false });
  f.assertClean();
});

test('geometry exceptions reject and cancel both schedulers', async () => {
  const f = fixture(() => { throw new Error('geometry unavailable'); });
  const result = f.injected.elementState(f.element, 'stable', 100);
  const rejection = assert.rejects(result, /geometry unavailable/);
  f.frame(0);
  f.advance(100);
  await rejection;
  f.assertClean();
});
