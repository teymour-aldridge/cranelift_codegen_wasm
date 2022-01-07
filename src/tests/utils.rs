use std::fmt;
use std::{path::Path, thread};

use cranelift_codegen::binemit::{NullStackMapSink, NullTrapSink};
use cranelift_codegen::{ir, Context};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::Module;
use cranelift_reader::parse_functions;
use log::LevelFilter;
use log4rs::{
    append::file::FileAppender,
    config::{Appender, Logger, Root},
    encode::pattern::PatternEncoder,
};
use walrus::ModuleConfig;
use wasmtime::{Config, Engine, Instance, Store, WasmParams, WasmResults};

use crate::WasmModule;

pub(crate) fn enable_log(unique: impl fmt::Display) {
    let output = FileAppender::builder()
        .encoder(Box::new(PatternEncoder::new("{d} - {m}{n}")))
        .build(format!("log/output-{}.log", unique))
        .unwrap();
    let config = log4rs::config::Config::builder()
        .appender(Appender::builder().build("logs", Box::new(output)))
        .logger(
            Logger::builder()
                .appender("logs")
                .build("logs", LevelFilter::Trace),
        )
        .build(Root::builder().appender("logs").build(LevelFilter::Trace))
        .unwrap();
    log4rs::init_config(config).unwrap();
}

pub(crate) fn run_test<Params: WasmParams, Return: WasmResults + std::fmt::Debug + Clone>(
    params: Params,
    sig: ir::Signature,
    build: impl FnOnce(&mut FunctionBuilder),
    check: impl FnOnce(Return) -> bool,
) {
    // todo: correct target isa
    let mut module = WasmModule::new(ModuleConfig::new());

    let func_id = module
        .declare_function("func_name", cranelift_module::Linkage::Export, &sig)
        .unwrap();

    let mut ctx = Context::new();
    ctx.func.signature.returns = sig.returns;

    let mut func_ctx = FunctionBuilderContext::new();
    let mut builder: FunctionBuilder = FunctionBuilder::new(&mut ctx.func, &mut func_ctx);

    (build)(&mut builder);

    builder.finalize();

    if std::env::var("PRINT_CLIF").is_ok() {
        println!("{}", ctx.func);
    }

    module
        .define_function(
            func_id,
            &mut ctx,
            &mut NullTrapSink {},
            &mut NullStackMapSink {},
        )
        .unwrap();

    if std::env::var("PRINT_WAT").is_ok() {
        println!("{}", module.emit_wat());
    }

    let wasm = module.emit();
    let engine = Engine::default();
    let module = wasmtime::Module::new(&engine, wasm).unwrap();
    let mut store = Store::new(&engine, ());
    let instance = Instance::new(&mut store, &module, &[]).unwrap();
    let func = instance
        .get_func(&mut store, "func_name")
        .expect("function not defined!");
    let func = func.typed::<Params, Return, _>(&store).unwrap();
    let ret = func.call(&mut store, params).unwrap();
    assert!(
        (check)(ret.clone()),
        "assertion failed\nnote: the return value was {:#?}",
        &ret
    )
}

/// Runs a test from a file.
///
/// Note that this will fail if the file takes longer than three seconds to run!
pub(crate) fn test_from_file<Params: WasmParams, Return: WasmResults + std::fmt::Debug + Clone>(
    params: Params,
    file: impl AsRef<Path>,
    check: impl FnOnce(Return) -> bool,
) {
    let file = ezio::file::read(file);

    let funcs = parse_functions(&file).unwrap();

    let func = funcs[0].clone();

    let mut module = WasmModule::new(ModuleConfig::new());

    let id = module
        .declare_function(
            "func_name",
            cranelift_module::Linkage::Export,
            &func.signature,
        )
        .unwrap();
    let mut ctx = Context::new();
    ctx.func = func;

    module
        .define_function(id, &mut ctx, &mut NullTrapSink {}, &mut NullStackMapSink {})
        .expect("failed to define function");

    if std::env::var("PRINT_WAT").is_ok() {
        println!("{}", module.emit_wat());
    }

    let wasm = module.emit();
    let engine = Engine::new(Config::new().interruptable(true)).unwrap();
    let module = wasmtime::Module::new(&engine, wasm).unwrap();
    let mut store = Store::new(&engine, ());

    let interrupt_handle = store.interrupt_handle().unwrap();

    let instance = Instance::new(&mut store, &module, &[]).unwrap();
    let func = instance
        .get_func(&mut store, "func_name")
        .expect("function not defined!");
    let func = func.typed::<Params, Return, _>(&store).unwrap();

    thread::spawn(move || {
        thread::sleep(std::time::Duration::from_secs(3));
        interrupt_handle.interrupt();
    });

    let ret = func.call(&mut store, params).unwrap();
    assert!(
        (check)(ret.clone()),
        "assertion failed\nnote: the return value was {:#?}",
        &ret
    );
}
