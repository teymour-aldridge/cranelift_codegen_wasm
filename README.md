# `cranelift_codegen_wasm`

Experimental code generation for WebAssembly from Cranelift IR.

**note: not ready for usage yet**

## Setup

Contains an item called `WasmModule` which implements
[`cranelift_module::Module`](https://docs.rs/cranelift-module/latest/cranelift_module/trait.Module.html)

## Todo

Everything!

(more specifically)
- lots of operators
- some ironing out of bugs in control-flow translation
- translation of more complex control flow
- testing

## Useful resources
- [Human-readable WebAssembly guide](https://github.com/sunfishcode/wasm-reference-manual/blob/master/WebAssembly.md)
- [Structured control flow problem](https://medium.com/leaningtech/solving-the-structured-control-flow-problem-once-and-for-all-5123117b1ee2)
