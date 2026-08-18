/**
 * Copyright (c) 2010-2014, Christian Johansen, christian@cjohansen.no. All rights reserved.
 * Modifications copyright (c) Microsoft Corporation.
 *
 * Redistribution and use in source and binary forms, with or without modification, are permitted provided that the following conditions are met:
 * 1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following disclaimer.
 * 2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the following disclaimer in the documentation and/or other materials provided with the distribution.
 * 3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote products derived from this software without specific prior written permission.
 *
 * THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
 */

/**
 * Adapted for Refact from Playwright `packages/injected/src/clock.ts` at commit
 * d5a185a894ab3ab17ff77a44e116a1339c6bdaed. TypeScript annotations are erased and the module is
 * wrapped in an idempotent installer publishing `globalThis.__refactClock`; timer semantics are
 * unchanged.
 */
(() => {
  if (globalThis.__refactClock)
    return;

  const TimerType = {
    Timeout: 'Timeout',
    Interval: 'Interval',
    Immediate: 'Immediate',
    AnimationFrame: 'AnimationFrame',
    IdleCallback: 'IdleCallback',
  };

  const maxTimeout = Math.pow(2, 31) - 1;  // see https://heycam.github.io/webidl/#abstract-opdef-converttoint
  const idCounterStart = 1e12; // arbitrarily large number to avoid collisions with native timer IDs

  function asWallTime(n) {
    return n;
  }

  function shiftTicks(ticks, ms) {
    return ticks + ms;
  }

  class ClockController {
    constructor(embedder) {
      this._duringTick = false;
      this._uniqueTimerId = idCounterStart;
      this.disposables = [];
      this._log = [];
      this._realTime = undefined;
      this._currentRealTimeTimer = undefined;
      this._timers = new Map();
      this._now = { time: asWallTime(0), isFixedTime: false, ticks: 0, origin: asWallTime(-1) };
      this._embedder = embedder;
    }

    uninstall() {
      this.disposables.forEach(dispose => dispose());
      this.disposables.length = 0;
    }

    now() {
      this._replayLogOnce();
      // Sync real time to support calling Date.now() in a loop.
      this._syncRealTime();
      return this._now.time;
    }

    install(time) {
      this._replayLogOnce();
      this._innerInstall(asWallTime(time));
    }

    setSystemTime(time) {
      this._replayLogOnce();
      this._innerSetTime(asWallTime(time));
    }

    setFixedTime(time) {
      this._replayLogOnce();
      this._innerSetFixedTime(asWallTime(time));
    }

    performanceNow() {
      this._replayLogOnce();
      // Sync real time to support calling performance.now() in a loop.
      this._syncRealTime();
      return this._now.ticks;
    }

    _syncRealTime() {
      if (!this._realTime)
        return;
      const now = this._embedder.performanceNow();
      const sinceLastSync = now - this._realTime.lastSyncTicks;
      if (sinceLastSync > 0) {
        this._advanceNow(shiftTicks(this._now.ticks, sinceLastSync));
        this._realTime.lastSyncTicks = now;
      }
    }

    _innerSetTime(time) {
      this._now.time = time;
      this._now.isFixedTime = false;
      if (this._now.origin < 0)
        this._now.origin = this._now.time;
    }

    _innerInstall(time) {
      // On a fresh install, reset the monotonic counter so that drift
      // accumulated by the realTime ticker before the user called install()
      // does not leak into performance.now().
      if (this._now.origin < 0)
        this._now.ticks = 0;
      this._innerSetTime(time);
    }

    _innerSetFixedTime(time) {
      this._innerSetTime(time);
      this._now.isFixedTime = true;
    }

    _advanceNow(to) {
      if (this._now.ticks > to) {
        // While running timers, `now` can advance by syncing with real time
        // from within now() or performance.now().
        // This makes it possible for `now` to be ahead of where we want to advance it.
        return;
      }
      if (!this._now.isFixedTime)
        this._now.time = asWallTime(this._now.time + to - this._now.ticks);
      this._now.ticks = to;
    }

    async log(type, time, param) {
      this._log.push({ type, time, param });
    }

    async runFor(ticks) {
      this._replayLogOnce();
      if (ticks < 0)
        throw new TypeError('Negative ticks are not supported');
      await this._runWithDisabledRealTimeSync(async () => {
        await this._runTo(shiftTicks(this._now.ticks, ticks));
      });
    }

    async _runTo(to) {
      to = Math.ceil(to);

      if (this._now.ticks > to)
        return;

      let firstException;
      while (true) {
        const result = await this._callFirstTimer(to);
        if (!result.timerFound)
          break;
        firstException = firstException || result.error;
      }

      this._advanceNow(to);

      if (firstException)
        throw firstException;
    }

    async pauseAt(time) {
      this._replayLogOnce();
      await this._innerPause();
      const toConsume = time - this._now.time;
      await this._innerFastForwardTo(shiftTicks(this._now.ticks, toConsume));
      return toConsume;
    }

    async _innerPause() {
      this._realTime = undefined;
      await this._currentRealTimeTimer?.dispose();
      this._currentRealTimeTimer = undefined;
    }

    resume() {
      this._replayLogOnce();
      this._innerResume();
    }

    _innerResume() {
      const now = this._embedder.performanceNow();
      this._realTime = { startTicks: now, lastSyncTicks: now };
      this._updateRealTimeTimer();
    }

    _updateRealTimeTimer() {
      if (this._currentRealTimeTimer?.promise) {
        // In progress, safe to return as it will call itself once promise is resolved.
        return;
      }

      const firstTimer = this._firstTimer();

      // Either run the next timer or move time in 100ms chunks.
      const nextTick = Math.min(firstTimer ? firstTimer.callAt : this._now.ticks + maxTimeout, this._now.ticks + 100);
      const callAt = this._currentRealTimeTimer ? Math.min(this._currentRealTimeTimer.callAt, nextTick) : nextTick;

      if (this._currentRealTimeTimer) {
        // Cancel and reschedule.
        this._currentRealTimeTimer.cancel();
        this._currentRealTimeTimer = undefined;
      }

      const realTimeTimer = {
        callAt,
        promise: undefined,
        cancel: this._embedder.setTimeout(() => {
          this._syncRealTime();
          realTimeTimer.promise = this._runTo(this._now.ticks).catch(e => console.error(e));
          void realTimeTimer.promise.then(() => {
            this._currentRealTimeTimer = undefined;
            if (this._realTime)
              this._updateRealTimeTimer();
          });
        }, callAt - this._now.ticks),
        dispose: async () => {
          realTimeTimer.cancel();
          await realTimeTimer.promise;
        }
      };

      this._currentRealTimeTimer = realTimeTimer;
    }

    async _runWithDisabledRealTimeSync(fn) {
      if (!this._realTime) {
        await fn();
        return;
      }

      await this._innerPause();
      try {
        await fn();
      } finally {
        this._innerResume();
      }
    }

    async fastForward(ticks) {
      this._replayLogOnce();
      await this._runWithDisabledRealTimeSync(async () => {
        await this._innerFastForwardTo(shiftTicks(this._now.ticks, ticks | 0));
      });
    }

    async _innerFastForwardTo(to) {
      if (to < this._now.ticks)
        throw new Error('Cannot fast-forward to the past');
      for (const timer of this._timers.values()) {
        if (to > timer.callAt)
          timer.callAt = to;
      }
      await this._runTo(to);
    }

    addTimer(options) {
      this._replayLogOnce();

      if (options.type === TimerType.AnimationFrame && !options.func)
        throw new Error('Callback must be provided to requestAnimationFrame calls');
      if (options.type === TimerType.IdleCallback && !options.func)
        throw new Error('Callback must be provided to requestIdleCallback calls');
      if ([TimerType.Timeout, TimerType.Interval].includes(options.type) && !options.func && options.delay === undefined)
        throw new Error('Callback must be provided to timer calls');

      let delay = options.delay ? +options.delay : 0;
      if (!Number.isFinite(delay))
        delay = 0;
      delay = delay > maxTimeout ? 1 : delay;
      delay = Math.max(0, delay);

      const timer = {
        type: options.type,
        func: options.func,
        args: options.args || [],
        delay,
        callAt: shiftTicks(this._now.ticks, (delay || (this._duringTick ? 1 : 0))),
        createdAt: this._now.ticks,
        id: this._uniqueTimerId++,
        error: new Error(),
      };
      this._timers.set(timer.id, timer);
      if (this._realTime)
        this._updateRealTimeTimer();
      return timer.id;
    }

    countTimers() {
      return this._timers.size;
    }

    _firstTimer(beforeTick) {
      let firstTimer = null;

      for (const timer of this._timers.values()) {
        const isInRange = beforeTick === undefined || timer.callAt <= beforeTick;
        if (isInRange && (!firstTimer || compareTimers(firstTimer, timer) === 1))
          firstTimer = timer;
      }
      return firstTimer;
    }

    _takeFirstTimer(beforeTick) {
      const timer = this._firstTimer(beforeTick);
      if (!timer)
        return null;

      this._advanceNow(timer.callAt);

      if (timer.type === TimerType.Interval)
        timer.callAt = shiftTicks(timer.callAt, timer.delay);
      else
        this._timers.delete(timer.id);
      return timer;
    }

    async _callFirstTimer(beforeTick) {
      const timer = this._takeFirstTimer(beforeTick);
      if (!timer)
        return { timerFound: false };

      this._duringTick = true;
      try {
        if (typeof timer.func !== 'function') {
          let error;
          try {
            // Using global this is not correct here,
            // but it is already broken since the eval scope is different from the one
            // on the original call site.
            (() => { globalThis.eval(timer.func); })();
          } catch (e) {
            error = e;
          }
          await new Promise(f => this._embedder.setTimeout(f));
          return { timerFound: true, error };
        }

        let args = timer.args;
        if (timer.type === TimerType.AnimationFrame)
          args = [this._now.ticks];
        else if (timer.type === TimerType.IdleCallback)
          args = [{ didTimeout: false, timeRemaining: () => 0 }];

        let error;
        try {
          timer.func.apply(null, args);
        } catch (e) {
          error = e;
        }
        await new Promise(f => this._embedder.setTimeout(f));
        return { timerFound: true, error };
      } finally {
        this._duringTick = false;
      }
    }

    getTimeToNextFrame() {
      // When `window.requestAnimationFrame` is the first call in the page,
      // this place is the first API call, so replay the log.
      this._replayLogOnce();
      return 16 - this._now.ticks % 16;
    }

    clearTimer(timerId, type) {
      this._replayLogOnce();

      if (!timerId) {
        // null appears to be allowed in most browsers, and appears to be
        // relied upon by some libraries, like Bootstrap carousel
        return;
      }

      // in Node, the ID is stored as the primitive value for `Timeout` objects
      // for `Immediate` objects, no ID exists, so it gets coerced to NaN
      const id = Number(timerId);

      if (Number.isNaN(id) || id < idCounterStart) {
        const handlerName = getClearHandler(type);
        new Error(`Clock: ${handlerName} was invoked to clear a native timer instead of one created by the clock library.`);
      }

      const timer = this._timers.get(id);
      if (timer) {
        if (
          timer.type === type ||
          (timer.type === 'Timeout' && type === 'Interval') ||
          (timer.type === 'Interval' && type === 'Timeout')
        ) {
          this._timers.delete(id);
        } else {
          const clear = getClearHandler(type);
          const schedule = getScheduleHandler(timer.type);
          throw new Error(
              `Cannot clear timer: timer created with ${schedule}() but cleared with ${clear}()`,
          );
        }
      }
    }

    _replayLogOnce() {
      if (!this._log.length)
        return;

      let lastLogTime = -1;
      let isPaused = false;

      for (const { type, time, param } of this._log) {
        if (!isPaused && lastLogTime !== -1)
          this._advanceNow(shiftTicks(this._now.ticks, time - lastLogTime));
        lastLogTime = time;

        if (type === 'install') {
          this._innerInstall(asWallTime(param));
        } else if (type === 'fastForward' || type === 'runFor') {
          this._advanceNow(shiftTicks(this._now.ticks, param));
        } else if (type === 'pauseAt') {
          isPaused = true;
          this._innerSetTime(asWallTime(param));
        } else if (type === 'resume') {
          isPaused = false;
        } else if (type === 'setFixedTime') {
          this._innerSetFixedTime(asWallTime(param));
        } else if (type === 'setSystemTime') {
          this._innerSetTime(asWallTime(param));
        }
      }

      if (!isPaused) {
        if (lastLogTime > 0)
          this._advanceNow(shiftTicks(this._now.ticks, this._embedder.dateNow() - lastLogTime));
        this._innerResume();
      } else {
        this._realTime = undefined;
      }

      this._log.length = 0;
    }
  }

  function mirrorDateProperties(target, source) {
    for (const prop in source) {
      if (source.hasOwnProperty(prop))
        target[prop] = source[prop];
    }
    target.toString = () => source.toString();
    target.prototype = source.prototype;
    target.parse = source.parse;
    target.UTC = source.UTC;
    target.prototype.toUTCString = source.prototype.toUTCString;
    target.isFake = true;
    return target;
  }

  function createDate(clock, NativeDate) {
    function ClockDate(year, month, date, hour, minute, second, ms) {
      // the Date constructor called as a function, ref Ecma-262 Edition 5.1, section 15.9.2.
      // This remains so in the 10th edition of 2019 as well.
      if (!(this instanceof ClockDate))
        return new NativeDate(clock.now()).toString();

      // if Date is called as a constructor with 'new' keyword
      // Defensive and verbose to avoid potential harm in passing
      // explicit undefined when user does not pass argument
      switch (arguments.length) {
        case 0:
          return new NativeDate(clock.now());
        case 1:
          return new NativeDate(year);
        case 2:
          return new NativeDate(year, month);
        case 3:
          return new NativeDate(year, month, date);
        case 4:
          return new NativeDate(year, month, date, hour);
        case 5:
          return new NativeDate(year, month, date, hour, minute);
        case 6:
          return new NativeDate(
              year,
              month,
              date,
              hour,
              minute,
              second,
          );
        default:
          return new NativeDate(
              year,
              month,
              date,
              hour,
              minute,
              second,
              ms,
          );
      }
    }

    ClockDate.now = () => clock.now();
    return mirrorDateProperties(ClockDate, NativeDate);
  }

  /**
   * Mirror Intl by default on our fake implementation
   *
   * Most of the properties are the original native ones,
   * but we need to take control of those that have a
   * dependency on the current clock.
   */
  function createIntl(clock, NativeIntl) {
    const ClockIntl = {};
    /*
      * All properties of Intl are non-enumerable, so we need
      * to do a bit of work to get them out.
      */
    for (const key of Object.getOwnPropertyNames(NativeIntl))
      ClockIntl[key] = NativeIntl[key];

    ClockIntl.DateTimeFormat = function(...args) {
      const realFormatter = new NativeIntl.DateTimeFormat(...args);
      const formatter = {
        formatRange: realFormatter.formatRange.bind(realFormatter),
        formatRangeToParts: realFormatter.formatRangeToParts.bind(realFormatter),
        resolvedOptions: realFormatter.resolvedOptions.bind(realFormatter),
        format: date => realFormatter.format(date || clock.now()),
        formatToParts: date => realFormatter.formatToParts(date || clock.now()),
      };

      return formatter;
    };

    ClockIntl.DateTimeFormat.prototype = Object.create(
        NativeIntl.DateTimeFormat.prototype,
    );

    ClockIntl.DateTimeFormat.supportedLocalesOf =
      NativeIntl.DateTimeFormat.supportedLocalesOf;

    return ClockIntl;
  }

  function compareTimers(a, b) {
    // Sort first by absolute timing
    if (a.callAt < b.callAt)
      return -1;
    if (a.callAt > b.callAt)
      return 1;

    // Sort next by immediate, immediate timers take precedence
    if (a.type === TimerType.Immediate && b.type !== TimerType.Immediate)
      return -1;
    if (a.type !== TimerType.Immediate && b.type === TimerType.Immediate)
      return 1;

    // Sort next by creation time, earlier-created timers take precedence
    if (a.createdAt < b.createdAt)
      return -1;
    if (a.createdAt > b.createdAt)
      return 1;

    // Sort next by id, lower-id timers take precedence
    if (a.id < b.id)
      return -1;
    if (a.id > b.id)
      return 1;

    // As timer ids are unique, no fallback `0` is necessary
  }

  function platformOriginals(globalObject) {
    const raw = {
      setTimeout: globalObject.setTimeout,
      clearTimeout: globalObject.clearTimeout,
      setInterval: globalObject.setInterval,
      clearInterval: globalObject.clearInterval,
      requestAnimationFrame: globalObject.requestAnimationFrame ? globalObject.requestAnimationFrame : undefined,
      cancelAnimationFrame: globalObject.cancelAnimationFrame ? globalObject.cancelAnimationFrame : undefined,
      requestIdleCallback: globalObject.requestIdleCallback ? globalObject.requestIdleCallback : undefined,
      cancelIdleCallback: globalObject.cancelIdleCallback ? globalObject.cancelIdleCallback : undefined,
      Date: globalObject.Date,
      performance: globalObject.performance,
      Intl: globalObject.Intl,
      AbortSignal: globalObject.AbortSignal,
    };
    const bound = { ...raw };
    for (const key of Object.keys(bound)) {
      if (key !== 'Date' && key !== 'AbortSignal' && typeof bound[key] === 'function')
        bound[key] = bound[key].bind(globalObject);
    }
    return { raw, bound };
  }

  /**
   * Gets schedule handler name for a given timer type
   */
  function getScheduleHandler(type) {
    if (type === 'IdleCallback' || type === 'AnimationFrame')
      return `request${type}`;

    return `set${type}`;
  }

  function createApi(clock, originals, browserName) {
    return {
      setTimeout: (func, timeout, ...args) => {
        const delay = timeout ? +timeout : timeout;
        return clock.addTimer({
          type: TimerType.Timeout,
          func,
          args,
          delay
        });
      },
      clearTimeout: timerId => {
        if (timerId)
          clock.clearTimer(timerId, TimerType.Timeout);
      },
      setInterval: (func, timeout, ...args) => {
        const delay = timeout ? +timeout : timeout;
        return clock.addTimer({
          type: TimerType.Interval,
          func,
          args,
          delay,
        });
      },
      clearInterval: timerId => {
        if (timerId)
          return clock.clearTimer(timerId, TimerType.Interval);
      },
      requestAnimationFrame: callback => {
        return clock.addTimer({
          type: TimerType.AnimationFrame,
          func: callback,
          delay: clock.getTimeToNextFrame(),
        });
      },
      cancelAnimationFrame: timerId => {
        if (timerId)
          return clock.clearTimer(timerId, TimerType.AnimationFrame);
      },
      requestIdleCallback: (callback, options) => {
        let timeToNextIdlePeriod = 0;

        if (clock.countTimers() > 0)
          timeToNextIdlePeriod = 50; // const for now
        return clock.addTimer({
          type: TimerType.IdleCallback,
          func: callback,
          delay: options?.timeout ? Math.min(options?.timeout, timeToNextIdlePeriod) : timeToNextIdlePeriod,
        });
      },
      cancelIdleCallback: timerId => {
        if (timerId)
          return clock.clearTimer(timerId, TimerType.IdleCallback);
      },
      Intl: originals.Intl ? createIntl(clock, originals.Intl) : undefined,
      Date: createDate(clock, originals.Date),
      performance: originals.performance ? fakePerformance(clock, originals.performance) : undefined,
      AbortSignal: originals.AbortSignal ? fakeAbortSignal(clock, originals.AbortSignal, browserName) : undefined,
    };
  }

  function getClearHandler(type) {
    if (type === 'IdleCallback' || type === 'AnimationFrame')
      return `cancel${type}`;

    return `clear${type}`;
  }

  class FakePerformanceEntry {
    constructor(name, entryType, startTime, duration) {
      this.name = name;
      this.entryType = entryType;
      this.startTime = startTime;
      this.duration = duration;
    }

    toJSON() {
      return JSON.stringify({ ...this });
    }
  }

  function fakePerformance(clock, performance) {
    const result = {
      now: () => clock.performanceNow(),
    };
    result.__defineGetter__('timeOrigin', () => clock._now.origin || 0);
    for (const key of Object.keys(performance.__proto__)) {
      if (key === 'now' || key === 'timeOrigin')
        continue;
      if (key === 'getEntries' || key === 'getEntriesByName' || key === 'getEntriesByType')
        result[key] = () => [];
      else if (key === 'mark')
        result[key] = name => new FakePerformanceEntry(name, 'mark', 0, 0);
      else if (key === 'measure')
        result[key] = name => new FakePerformanceEntry(name, 'measure', 0, 50);
      else
        result[key] = () => {};
    }
    return result;
  }

  function fakeAbortSignal(clock, abortSignal, browserName) {
    Object.defineProperty(abortSignal, 'timeout', {
      value(ms) {
        const controller = new AbortController();
        clock.addTimer({
          delay: ms,
          type: TimerType.Timeout,
          func: () => controller.abort(
              new DOMException(
                  browserName === 'chromium' ? 'signal timed out' : 'The operation timed out.',
                  'TimeoutError'
              )
          ),
        });
        return controller.signal;
      }
    });
    return abortSignal;
  }

  function createClock(globalObject, config = {}) {
    const originals = platformOriginals(globalObject);
    const embedder = {
      dateNow: () => originals.raw.Date.now(),
      performanceNow: () => Math.ceil(originals.raw.performance.now()),
      setTimeout: (task, timeout) => {
        const timerId = originals.bound.setTimeout(task, timeout);
        return () => originals.bound.clearTimeout(timerId);
      },
      setInterval: (task, delay) => {
        const intervalId = originals.bound.setInterval(task, delay);
        return () => originals.bound.clearInterval(intervalId);
      },
    };

    const clock = new ClockController(embedder);
    const api = createApi(clock, originals.bound, config.browserName);
    return { clock, api, originals: originals.raw };
  }

  function install(globalObject, config = {}) {
    if (globalObject.Date?.isFake) {
      // Timers are already faked; this is a problem.
      // Make the user reset timers before continuing.
      throw new TypeError(`Can't install fake timers twice on the same global object.`);
    }

    const { clock, api, originals } = createClock(globalObject, config);
    const toFake = config.toFake?.length ? config.toFake : Object.keys(originals);

    for (const method of toFake) {
      if (method === 'Date') {
        globalObject.Date = mirrorDateProperties(api.Date, globalObject.Date);
      } else if (method === 'Intl') {
        globalObject.Intl = api[method];
      } else if (method === 'AbortSignal') {
        globalObject.AbortSignal = api[method];
      } else if (method === 'performance') {
        globalObject.performance = api[method];
        const kEventTimeStamp = Symbol('refactEventTimeStamp');
        Object.defineProperty(Event.prototype, 'timeStamp', {
          get() {
            if (!this[kEventTimeStamp])
              this[kEventTimeStamp] = api.performance?.now();
            return this[kEventTimeStamp];
          }
        });
      } else {
        globalObject[method] = (...args) => {
          return api[method].apply(api, args);
        };
      }
      clock.disposables.push(() => {
        globalObject[method] = originals[method];
      });
    }

    return { clock, api, originals };
  }

  function inject(globalObject, browserName) {
    const builtins = platformOriginals(globalObject).bound;
    const { clock: controller } = install(globalObject, { browserName });
    controller.resume();
    return {
      controller,
      builtins,
    };
  }

  globalThis.__refactClock = inject(globalThis, 'chromium');
})();
