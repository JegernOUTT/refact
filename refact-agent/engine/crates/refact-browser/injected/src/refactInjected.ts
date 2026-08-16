/**
 * Copyright 2026 SmallCloud Technologies Ltd.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 * http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

import {
  type CheckedState,
  getAriaDisabled,
  getCheckedState,
  getReadonly,
  isElementVisible,
} from './vendor/injected/domUtils';

type RefactBuiltins = Readonly<Record<string, unknown>>;

type ElementStateName = 'visible' | 'enabled' | 'editable' | 'checked' | 'unchecked' | 'mixed' | 'stable';

type ElementStates = Readonly<{
  visible: boolean;
  enabled: boolean;
  editable: boolean | null;
  checked: CheckedState | null;
  stable: boolean;
}>;

type RefactLocator = Readonly<{
  by: string;
  value?: string;
  role?: string;
  name?: string;
  exact?: boolean;
  nth?: number;
  within?: string;
}>;

const injectedInstanceName = '__refact_injected__';
const bindingName = '__refact_binding';

type RefactGlobal = typeof globalThis & {
  [injectedInstanceName]?: RefactInjected;
  [bindingName]?: (payload: string) => void;
};

export class RefactInjected {
  private readonly global: typeof globalThis;
  private readonly builtinSnapshot: RefactBuiltins;

  constructor(global: typeof globalThis, builtins: RefactBuiltins) {
    this.global = global;
    this.builtinSnapshot = builtins;
  }

  version(): string {
    return 'playwright-1.63.0-next-refact-1';
  }

  builtins(): RefactBuiltins {
    return this.builtinSnapshot;
  }

  resolveSimple(cssSelector: string): Element | null {
    return this.global.document.querySelector(cssSelector);
  }

  resolveAll(locator: RefactLocator): Element[] {
    const document = this.global.document;
    const scope = locator.within ? document.querySelector(locator.within) : document;
    if (!scope)
      throw new Error('Scope selector not found');
    let elements: Element[];
    switch (locator.by) {
      case 'css':
        elements = Array.from(scope.querySelectorAll(locator.value ?? ''));
        break;
      case 'id': {
        const element = scope.querySelector(`#${CSS.escape(locator.value ?? '')}`);
        elements = element ? [element] : [];
        break;
      }
      case 'name':
        elements = Array.from(scope.querySelectorAll(`[name=${JSON.stringify(locator.value ?? '')}]`));
        break;
      case 'test_id':
        elements = Array.from(scope.querySelectorAll(`[data-testid=${JSON.stringify(locator.value ?? '')}]`));
        break;
      case 'placeholder':
        elements = Array.from(scope.querySelectorAll(`[placeholder=${JSON.stringify(locator.value ?? '')}]`));
        break;
      case 'autocomplete':
        elements = Array.from(scope.querySelectorAll(`[autocomplete=${JSON.stringify(locator.value ?? '')}]`));
        break;
      case 'text': {
        const target = locator.value ?? '';
        elements = Array.from(scope.querySelectorAll('*')).filter(element => {
          const text = (element as HTMLElement).innerText;
          return locator.exact ? text?.trim() === target : !!text?.includes(target);
        });
        break;
      }
      case 'label': {
        const target = locator.value ?? '';
        elements = [];
        for (const label of Array.from(scope.querySelectorAll('label'))) {
          if (!(label as HTMLElement).innerText?.trim().includes(target))
            continue;
          const element = label.htmlFor
            ? document.getElementById(label.htmlFor)
            : label.querySelector('input,textarea,select');
          if (element)
            elements.push(element);
        }
        if (!elements.length)
          elements = Array.from(scope.querySelectorAll('[aria-label]')).filter(element =>
            element.getAttribute('aria-label')?.includes(target),
          );
        break;
      }
      case 'role': {
        const role = locator.role ?? '';
        const candidates = Array.from(scope.querySelectorAll(`[role=${JSON.stringify(role)}]`));
        elements = locator.name
          ? candidates.filter(element => {
            const name = element.getAttribute('aria-label') || (element as HTMLElement).innerText || '';
            return name.trim().includes(locator.name ?? '');
          })
          : candidates;
        break;
      }
      case 'xpath': {
        const result = document.evaluate(
          locator.value ?? '',
          scope,
          null,
          XPathResult.ORDERED_NODE_SNAPSHOT_TYPE,
          null,
        );
        elements = [];
        for (let index = 0; index < result.snapshotLength; index++) {
          const element = result.snapshotItem(index);
          if (element instanceof Element)
            elements.push(element);
        }
        break;
      }
      default:
        throw new Error(`Unknown locator strategy: ${locator.by}`);
    }
    if (locator.nth !== undefined)
      elements = elements.length > locator.nth ? [elements[locator.nth]] : [];
    return elements;
  }

  async elementState(element: Element, state: ElementStateName): Promise<Record<string, unknown>> {
    this.ensureConnected(element);
    if (state === 'visible') {
      const visible = isElementVisible(element);
      return { visible, matches: visible };
    }
    if (state === 'enabled') {
      const enabled = !getAriaDisabled(element);
      return { enabled, matches: enabled };
    }
    if (state === 'editable') {
      const editable = this.editableState(element);
      if (editable === null)
        throw new Error('Element is not an <input>, <textarea>, <select> or [contenteditable] and does not have a role allowing [aria-readonly]');
      return { editable, matches: editable };
    }
    if (state === 'checked' || state === 'unchecked' || state === 'mixed') {
      const checked = getCheckedState(element);
      if (checked === null)
        throw new Error('Not a checkbox or radio button');
      return {
        checked,
        matches: state === 'checked' ? checked === 'checked' : state === 'unchecked' ? checked === 'unchecked' : checked === 'mixed',
      };
    }
    if (state === 'stable') {
      const stable = await this.checkElementIsStable(element);
      return { stable, matches: stable };
    }
    throw new Error(`Unexpected element state "${state}"`);
  }

  async elementStates(element: Element): Promise<ElementStates> {
    this.ensureConnected(element);
    return {
      visible: isElementVisible(element),
      enabled: !getAriaDisabled(element),
      editable: this.editableState(element),
      checked: getCheckedState(element),
      stable: await this.checkElementIsStable(element),
    };
  }

  private editableState(element: Element): boolean | null {
    const readonly = getReadonly(element);
    return readonly === 'error' ? null : !getAriaDisabled(element) && !readonly;
  }

  private ensureConnected(element: Element): void {
    if (!element || !element.isConnected)
      throw new Error('Element is not attached to the DOM');
  }

  private async checkElementIsStable(element: Element): Promise<boolean> {
    const requestAnimationFrame = this.builtinSnapshot.requestAnimationFrame as (callback: FrameRequestCallback) => number;
    const performanceNow = this.builtinSnapshot.performanceNow as () => number;
    let lastRect: { x: number; y: number; width: number; height: number } | undefined;
    let lastTime = 0;
    return await new Promise<boolean>((resolve, reject) => {
      const check = () => {
        try {
          this.ensureConnected(element);
          const time = performanceNow();
          if (lastTime && time - lastTime < 15) {
            requestAnimationFrame(check);
            return;
          }
          lastTime = time;
          const clientRect = element.getBoundingClientRect();
          const rect = { x: clientRect.x, y: clientRect.y, width: clientRect.width, height: clientRect.height };
          if (lastRect) {
            resolve(
              rect.x === lastRect.x &&
              rect.y === lastRect.y &&
              rect.width === lastRect.width &&
              rect.height === lastRect.height,
            );
            return;
          }
          lastRect = rect;
          requestAnimationFrame(check);
        } catch (error) {
          reject(error);
        }
      };
      requestAnimationFrame(check);
    });
  }

  dispatchBinding(name: string, payload: unknown): void {
    const global = this.global as RefactGlobal;
    const binding = global[bindingName];
    const stringify = this.builtinSnapshot.jsonStringify as (value: unknown) => string;
    if (!binding)
      throw new Error(`${bindingName} is not installed`);
    binding(stringify({ name, payload }));
  }
}

export function bootstrapRefactInjected(
  global: typeof globalThis,
  builtins: RefactBuiltins,
): RefactInjected {
  const refactGlobal = global as RefactGlobal;
  const existing = refactGlobal[injectedInstanceName];
  if (existing)
    return existing;
  const injected = new RefactInjected(global, builtins);
  refactGlobal[injectedInstanceName] = injected;
  return injected;
}
