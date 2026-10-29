import { describe, expect, it } from "vitest";
import { readFileSync, readdirSync } from "node:fs";
import { staticSpzBindings } from "../../scripts/spz-csp-plugin.mjs";

const dir = "node_modules/@playcanvas/splat-transform/dist";
const codec = readFileSync(`${dir}/${readdirSync(dir).find(f => /^spz-.*\.js$/.test(f))}`, "utf8");
const adapted = staticSpzBindings(codec);

describe("offline codec security", () => {
  it("adapts both fixed codec builds without changing the WASM prefix or allowing JavaScript eval", () => {
    for (const file of readdirSync(dir).filter(f => /^spz-.*\.js$/.test(f))) {
      const original = readFileSync(`${dir}/${file}`, "utf8"), result = staticSpzBindings(original);
      expect(result.startsWith(original.slice(0, original.indexOf("function createJsInvoker(")))).toBe(true);
      expect(result).not.toMatch(/new Function\b|\beval\s*\(/);
    }
    expect(() => staticSpzBindings("changed codec")).toThrow("review the CSP adapter");
    const config = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8"));
    expect(config.app.security.csp).not.toContain("'unsafe-eval'");
    expect(config.app.security.csp).toContain("'wasm-unsafe-eval'");
    expect(readFileSync("src/workers/gaussianExport.worker.ts", "utf8")).toContain('webp.wasm?url');
  });
  it("preserves generated invoker argument conversion, return values and destruction", () => {
    const get = (code: string, stack: boolean) => {
      const source = code.slice(code.indexOf("function createJsInvoker("), code.indexOf("function craftInvokerFunction("));
      // The original JIT is evaluated in the test process only, as a reference.
      return new Function("usesDestructorStack", `${source};return createJsInvoker;`)(() => stack);
    };
    for (const stack of [false, true]) for (const count of [0, 1, 4]) for (const returns of [false, true]) {
      const run = (code: string) => {
        const log: unknown[] = [];
        const types = [{ isVoid: !returns }, null, ...Array.from({ length: count }, (_, i) => ({ destructorFunction: i % 2 === 0 ? () => {} : null }))];
        const args = ["name", () => {}, (...values: number[]) => { log.push(values); return 42; }, 7, (d: number[]) => log.push(d), (v: number) => v * 2, null];
        for (let i = 0; i < count; i++) args.push(((d: unknown[] | null, v: number) => { d?.push(v); return v + 10; }) as never);
        if (!stack) for (let i = 0; i < count; i += 2) args.push(((v: number) => log.push(["destroy", v])) as never);
        const fn = get(code, stack)(types, false, returns, false)(...args);
        return { value: fn(...Array.from({ length: count }, (_, i) => i)), log };
      };
      expect(run(adapted)).toEqual(run(codec));
    }
  });
  it("preserves emval function, method this-binding, constructor and cast invokers", () => {
    for (const kind of [0, 1, 2, 3]) for (const isVoid of [false, true]) {
      const run = (code: string) => {
        const start = code.indexOf("var __emval_create_invoker="), end = code.indexOf("function __emval_get_global(", start);
        const body = code.slice(start, end);
        const log: unknown[] = [];
        const types = [{ isVoid, toWireType: (_d: unknown, v: unknown) => v }, { name: "int", readValueFromPointer: (p: number) => p }];
        const emval = { toValue: () => kind === 0 ? ((n: number) => n + 5) : kind === 1 ? { base: 8, call(n: number) { return this.base + n; } } : class Value { constructor(public n: number) {} } };
        const fn = new Function("emval_lookupTypes", "Emval", "getStringOrSymbol", "emval_returnValue", "emval_addMethodCaller", "createNamedFunction", `${body};return __emval_create_invoker;`)(
          () => types, emval, () => "call", (wire: (d: unknown, value: unknown) => unknown, ref: number, value: unknown) => { log.push(ref); return wire(null, value); }, (f: unknown) => f, (_name: string, f: unknown) => f,
        )(2, 0, kind);
        const value = fn(0, 0, 19, 100);
        return { value: typeof value === "object" ? value?.n : value, log };
      };
      expect(run(adapted)).toEqual(run(codec));
    }
  });
});
