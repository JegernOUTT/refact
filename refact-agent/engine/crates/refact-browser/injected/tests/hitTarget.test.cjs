const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');
const bundle = fs.readFileSync(path.join(__dirname, '../../src/generated/injected_bundle.js'), 'utf8');

function fixture(action = 'mouse') {
  let now = 0, next = 0, hit;
  const timers = new Map(), listeners = new Map();
  const forbidden = () => { throw new Error('page timer must not be used'); };
  const document = { nodeType: 9 };
  const target = { isConnected: true, nodeType: 1, parentNode: document };
  const overlay = { nodeType: 1, nodeName: 'DIV', attributes: [], childNodes: [], textContent: '' };
  hit = target;
  const builtins = {
    Map, Set, WeakMap, arrayFrom: Array.from,
    performanceNow: () => now,
    setTimeout(callback, delay) { const id = ++next; timers.set(id, { callback, at: now + delay }); return id; },
    clearTimeout: id => timers.delete(id),
    documentElementsFromPoint: () => [hit], documentElementFromPoint: () => hit,
    addWindowEventListener(name, listener) { listeners.set(name, listener); },
    removeWindowEventListener(name) { listeners.delete(name); },
    preventDefault: event => { event.blocked = true; },
    stopPropagation() {}, stopImmediatePropagation() {},
  };
  const context = vm.createContext({ module: { exports: {} } });
  vm.runInContext(bundle, context);
  const injected = context.module.exports.bootstrapRefactInjected()({ document, setTimeout: forbidden, clearTimeout: forbidden }, builtins);
  const installed = injected.installHitTargetInterceptor(target, action);
  assert.equal(installed.status, 'installed');
  return {
    injected, target,
    emit(type, trusted = true, wrong = false, wrongPath = wrong) {
      hit = wrong ? overlay : target;
      const event = { type, isTrusted: trusted, clientX: 10, clientY: 20, composedPath: () => [wrongPath ? overlay : target, document] };
      if (type.startsWith('touch')) { event.touches = []; event.changedTouches = [{ clientX: 10, clientY: 20 }]; }
      listeners.get(type)?.(event);
      return event;
    },
    take: timeout => injected.takeHitTargetInterceptor(installed.id, timeout),
    advance(time) {
      for (;;) {
        const entry = [...timers].filter(([, timer]) => timer.at <= time).sort((a, b) => a[1].at - b[1].at)[0];
        if (!entry) break;
        now = entry[1].at; timers.delete(entry[0]); entry[1].callback();
      }
      now = time;
    },
    clean() { assert.equal(listeners.size, 0, 'listeners removed'); assert.equal(timers.size, 0, 'timers cancelled'); },
  };
}

for (const sequence of [[], ['pointerdown', 'mousedown'], ['mousedown', 'mouseup']]) {
  test(`missing terminal evidence is unknown: ${sequence.join(',') || 'zero'}`, async () => {
    const f = fixture(); sequence.forEach(type => f.emit(type));
    const result = f.take(); f.advance(100);
    assert.equal((await result).status, 'effect_unknown'); f.clean();
  });
}
for (const [action, terminal] of [['mouse', 'click'], ['mouse', 'auxclick'], ['mouse', 'dblclick'], ['mouse', 'contextmenu'], ['hover', 'mousemove'], ['tap', 'touchend'], ['tap', 'pointerup']]) {
  test(`${action} accepts trusted ${terminal}`, async () => {
    const f = fixture(action); f.emit(terminal);
    assert.equal((await f.take()).status, 'done'); f.clean();
  });
}
test('synthetic terminal does not establish delivery', async () => {
  const f = fixture(); f.emit('click', false);
  const result = f.take(); f.advance(100);
  assert.equal((await result).status, 'effect_unknown'); f.clean();
});
test('delayed terminal is observed within bounded native drain without redispatch', async () => {
  const f = fixture(); f.emit('mousedown'); const result = f.take();
  f.advance(40); const delivered = f.emit('click'); assert.equal(delivered.blocked, undefined); f.advance(100);
  assert.equal((await result).status, 'done'); f.clean();
});
test('later wrong-target event after progress is unknown, latched and blocked', async () => {
  const f = fixture(); f.emit('mousedown'); assert.equal(f.emit('mouseup', true, true).blocked, true); f.emit('click');
  assert.equal((await f.take()).status, 'effect_unknown'); f.clean();
});
test('initial intercepted event stays safely retryable', async () => {
  const f = fixture(); assert.equal(f.emit('mousedown', true, true).blocked, true);
  assert.equal((await f.take()).status, 'intercepted'); f.clean();
});
test('tap cancellation cannot become success', async () => {
  const f = fixture('tap'); f.emit('pointerdown'); f.emit('pointercancel'); f.emit('touchend');
  assert.equal((await f.take()).status, 'effect_unknown'); f.clean();
});

test('expired outer budget does not schedule a fresh drain', async () => {
  const f = fixture();
  assert.equal((await f.take(0)).status, 'effect_unknown'); f.clean();
});
test('geometric hit without event delivery to target is not success', async () => {
  const f = fixture();
  f.emit('mousedown'); f.emit('click', true, false, true);
  // An event retargeted elsewhere cannot prove intended delivery even at the same point.
  const result = f.take(0);
  assert.equal((await result).status, 'effect_unknown'); f.clean();
});

test('short remaining budget bounds drain and late events cannot resurrect settlement', async () => {
  const f = fixture(); const result = f.take(7); f.advance(7);
  assert.equal((await result).status, 'effect_unknown'); f.clean();
  f.emit('click'); f.advance(1000); f.clean();
});

for (const [name, sequence, expected] of [
  ['left click', ['pointerdown', 'mousedown', 'pointerup', 'mouseup', 'click'], 'done'],
  ['middle click', ['mousedown', 'mouseup', 'auxclick'], 'done'],
  ['right click', ['mousedown', 'contextmenu', 'mouseup'], 'done'],
  ['double click', ['mousedown', 'mouseup', 'click', 'mousedown', 'mouseup', 'click', 'dblclick'], 'done'],
  ['partial second click', ['mousedown', 'mouseup', 'click', 'mousedown', 'mouseup'], 'effect_unknown'],
]) {
  test(`${name} sequence preserves generic mouse semantics`, async () => {
    const f = fixture(); sequence.forEach(type => f.emit(type));
    const result = f.take(); f.advance(100);
    assert.equal((await result).status, expected); f.clean();
  });
}

for (const timeout of [0, 100]) {
  test(`double-click first click alone is unknown with budget ${timeout}`, async () => {
    const f = fixture('dblclick');
    ['mousedown', 'mouseup', 'click'].forEach(type => f.emit(type));
    const result = f.take(timeout); f.advance(100);
    assert.equal((await result).status, 'effect_unknown'); f.clean();
  });
}
for (const [name, sequence, expected] of [
  ['complete', ['mousedown', 'mouseup', 'click', 'dblclick'], 'done'],
  ['partial', ['mousedown', 'mouseup', 'click'], 'effect_unknown'],
]) {
  test(`double-click delayed second sequence ${name}`, async () => {
    const f = fixture('dblclick');
    ['mousedown', 'mouseup', 'click'].forEach(type => f.emit(type));
    const result = f.take(); f.advance(40);
    sequence.forEach(type => f.emit(type)); f.advance(100);
    assert.equal((await result).status, expected); f.clean();
  });
}
test('double-click later interception after first click is unknown', async () => {
  const f = fixture('dblclick');
  ['mousedown', 'mouseup', 'click'].forEach(type => f.emit(type));
  const result = f.take(); f.advance(40);
  assert.equal(f.emit('mousedown', true, true).blocked, true); f.advance(100);
  assert.equal((await result).status, 'effect_unknown'); f.clean();
});
