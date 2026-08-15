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

type RefactBuiltins = Readonly<Record<string, unknown>>;

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
