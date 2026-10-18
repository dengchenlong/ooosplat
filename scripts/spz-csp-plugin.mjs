// CSP adapter for the Emscripten glue shipped in splat-transform 3.10.1.
// Keep the WASM/codec unchanged; replace the two JS JIT binding factories with
// closure-based equivalents so the app never needs script-src 'unsafe-eval'.
// Unknown glue fails the build rather than silently relaxing CSP.
export function staticSpzBindings(code) {
  const replace = (start, end, replacement) => {
    const a = code.indexOf(start), b = code.indexOf(end, a);
    if (a < 0 || b < a || code.indexOf(start, a + start.length) !== -1) throw new Error("SPZ glue changed; review the CSP adapter before upgrading");
    code = code.slice(0, a) + replacement + code.slice(b);
  };
  replace("function createJsInvoker(", "function craftInvokerFunction(", `
function createJsInvoker(argTypes,isClassMethodFunc,returns,isAsync){
  const needs=usesDestructorStack(argTypes),count=argTypes.length-2;
  return function(humanName,throwBindingError,invoker,fn,runDestructors,fromRetWire,toClassParamWire,...captures){
    return function(...args){
      const destructors=needs?[]:null,wired=[];
      for(let i=0;i<count;i++)wired.push(captures[i](destructors,args[i]));
      const result=invoker(fn,...wired);
      if(needs)runDestructors(destructors);
      else{let d=count;for(let i=2;i<argTypes.length;i++)if(argTypes[i].destructorFunction!==null)captures[d++](wired[i-2]);}
      if(returns)return fromRetWire(result);
    };
  };
}
`);
  replace("var __emval_create_invoker=", "function __emval_get_global(", `
var __emval_create_invoker=function(argCount,argTypesPtr,kind){
  argTypesPtr>>>=0;
  const [retType,...argTypes]=emval_lookupTypes(argCount,argTypesPtr);
  const toReturnWire=retType.toWireType.bind(retType);
  const readers=argTypes.map(type=>type.readValueFromPointer.bind(type));
  const invoke=function(handle,methodName,destructorsRef,args){
    const values=readers.map((read,i)=>read(args+i*8));
    let value;
    if(kind===0)value=Emval.toValue(handle)(...values);
    else if(kind===1){const target=Emval.toValue(handle);value=target[getStringOrSymbol(methodName)](...values);}
    else if(kind===2)value=Reflect.construct(Emval.toValue(handle),values);
    else if(kind===3)value=values.at(-1);
    else throw new Error('Unsupported SPZ binding kind');
    if(!retType.isVoid)return emval_returnValue(toReturnWire,destructorsRef,value);
  };
  const name='methodCaller<('+argTypes.map(t=>t.name)+') => '+retType.name+'>';
  return emval_addMethodCaller(createNamedFunction(name,invoke));
};
`);
  if (/new Function\b|\beval\s*\(/.test(code)) throw new Error("Unexpected dynamic evaluation in SPZ glue");
  return code;
}
export function spzCspPlugin() {
  return {
    name: "ooosplat-static-spz-bindings",
    enforce: "pre",
    transform(code, id) {
      if (/[\\/]@playcanvas[\\/]splat-transform[\\/]dist[\\/]spz-[^/\\]+\.js$/.test(id)) return { code: staticSpzBindings(code), map: null };
    },
  };
}
