use std::str::FromStr;

use cranelift_codegen::{
    binemit::{NullStackMapSink, NullTrapSink},
    ir::{self, AbiParam, InstBuilder},
    settings, Context,
};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::Module;
use target_lexicon::triple;
use walrus::ModuleConfig;
use wasmtime::{Engine, Instance, Store};

use crate::WasmModule;

#[test]
/// Test that a function which returns only a simple constant can be compiled.
fn simple_const() {
    let builder = settings::builder();
    let shared_flags = settings::Flags::new(builder);
    // todo: correct target isa
    let mut module = WasmModule::new(
        ModuleConfig::new(),
        cranelift_codegen::isa::lookup(triple!("x86_64"))
            .unwrap()
            .finish(shared_flags),
    );

    let sig = ir::Signature {
        params: vec![],
        returns: vec![AbiParam::new(ir::types::I32)],
        call_conv: cranelift_codegen::isa::CallConv::SystemV,
    };

    let func_id = module
        .declare_function("constant", cranelift_module::Linkage::Export, &sig)
        .unwrap();

    let mut ctx = Context::new();
    ctx.func.signature.returns = vec![AbiParam::new(ir::types::I32)];

    let mut func_ctx = FunctionBuilderContext::new();
    {
        let mut builder: FunctionBuilder = FunctionBuilder::new(&mut ctx.func, &mut func_ctx);

        let entry_block = builder.create_block();
        builder.switch_to_block(entry_block);
        let ret_value = builder.ins().iconst(ir::types::I32, 42);
        builder.ins().return_(&[ret_value]);
        builder.seal_block(entry_block);

        builder.finalize();
    }

    module
        .define_function(
            func_id,
            &mut ctx,
            &mut NullTrapSink {},
            &mut NullStackMapSink {},
        )
        .unwrap();

    let wasm = module.emit();
    let engine = Engine::default();
    let module = wasmtime::Module::new(&engine, wasm).unwrap();
    let mut store = Store::new(&engine, ());
    let instance = Instance::new(&mut store, &module, &[]).unwrap();
    let func = instance
        .get_func(&mut store, "constant")
        .expect("function not defined!");
    let func = func.typed::<(), i32, _>(&store).unwrap();
    let res = func.call(&mut store, ()).unwrap();

    assert_eq!(res, 42);
}

#[test]
/// Test that it is possible to declare and use some data.
fn test_simple_data_decl() {
    todo!()
}

#[test]
/// Test some basic usage of the relooper algorithm.
fn test_simple_control_flow() {
    todo!()
}

#[test]
/// Test some more elaborate usage of the relooper algorithm.
fn test_elaborate_control_flow() {
    todo!()
}

#[test]
/// Test a series of files (e.g. wasm spec tests, some files from the Wasmtime repository).
fn test_various_clif_files() {
    todo!()
}
