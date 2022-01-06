use std::{fmt, path::Path, thread};

use cranelift_codegen::{
    binemit::{NullStackMapSink, NullTrapSink},
    ir::{self, condcodes::IntCC, AbiParam, InstBuilder},
    isa::CallConv,
    Context,
};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_module::Module;
use cranelift_reader::parse_functions;
use log::LevelFilter;
use log4rs::{
    append::file::FileAppender,
    config::{Appender, Logger, Root},
    encode::pattern::PatternEncoder,
};
use rusty_fork::rusty_fork_test;
use walrus::ModuleConfig;
use wasmtime::{Config, Engine, Instance, Store, WasmParams, WasmResults};

use crate::WasmModule;

fn enable_log(unique: impl fmt::Display) {
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
fn run_test<Params: WasmParams, Return: WasmResults + std::fmt::Debug + Clone>(
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

#[test]
/// Test that a function which returns only a simple constant can be compiled.
fn simple_const() {
    run_test(
        (),
        {
            ir::Signature {
                params: vec![],
                returns: vec![AbiParam::new(ir::types::I32)],
                call_conv: CallConv::SystemV,
            }
        },
        |builder| {
            let entry_block = builder.create_block();
            builder.switch_to_block(entry_block);
            let ret_value = builder.ins().iconst(ir::types::I32, 42);
            builder.ins().return_(&[ret_value]);
            builder.seal_block(entry_block);
        },
        |res: i32| -> bool { res == 42 },
    );
}

#[test]
fn test_simple_binop() {
    run_test(
        (),
        ir::Signature {
            params: vec![],
            returns: vec![AbiParam::new(ir::types::I32)],
            call_conv: CallConv::SystemV,
        },
        |builder| {
            let entry = builder.create_block();
            builder.switch_to_block(entry);
            let a = builder.ins().iconst(ir::types::I32, 1500);
            let b = builder.ins().iconst(ir::types::I32, 1500);

            let ret = builder.ins().iadd(a, b);
            builder.ins().return_(&[ret]);
            builder.seal_block(entry);
        },
        |res: i32| -> bool { res == 3000 },
    )
}

#[test]
/// Test some basic usage of the relooper algorithm.
fn test_simple_control_flow() {
    run_test(
        (),
        ir::Signature {
            params: vec![],
            returns: vec![AbiParam::new(ir::types::I32)],
            call_conv: CallConv::SystemV,
        },
        // this function looks roughly like:
        // i = 100
        // while i != 0 do
        //     i -= 1
        // endwhile
        // return i
        |builder| {
            let entry = builder.create_block();
            builder.switch_to_block(entry);

            let zero = builder.ins().iconst(ir::types::I32, 0);
            builder.declare_var(Variable::with_u32(0), ir::types::I32);
            let iteration_val = builder.ins().iconst(ir::types::I32, 100);
            let add_res = builder.ins().iadd(zero, iteration_val);
            builder.def_var(Variable::with_u32(0), add_res);

            let header_block = builder.create_block();
            let body_block = builder.create_block();
            let exit_block = builder.create_block();

            builder.ins().jump(header_block, &[]);
            builder.switch_to_block(header_block);
            let iteration = builder.use_var(Variable::with_u32(0));
            let condition = builder.ins().icmp(IntCC::Equal, iteration, zero);
            builder.ins().brz(condition, exit_block, &[]);
            builder.ins().jump(body_block, &[]);
            builder.switch_to_block(body_block);
            builder.seal_block(body_block);

            let one = builder.ins().iconst(ir::types::I32, 1);
            let sub_res = builder.ins().isub(iteration, one);
            builder.def_var(Variable::with_u32(0), sub_res);
            builder.ins().jump(header_block, &[]);

            builder.switch_to_block(exit_block);
            builder.seal_block(header_block);
            builder.seal_block(exit_block);
            builder.ins().return_(&[iteration]);
            builder.seal_block(entry);
        },
        |res: i32| -> bool { res == 0 },
    );
}

/// Runs a test from a file.
///
/// Note that this will fail if the file takes longer than three seconds to run!
fn test_from_file<Params: WasmParams, Return: WasmResults + std::fmt::Debug + Clone>(
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

#[test]
fn test_branching_from_file() {
    test_from_file(
        (0, 13),
        "src/filetests/wasmtime/branching.clif",
        |out: i32| out == 0,
    )
}

rusty_fork_test! {
#[test]
fn test_fibonacci_from_file() {
    enable_log("test_fibonacci_from_file");

    fn fib(n: i32) -> i32 {
        match n {
            0 | 1 | 2 => 1,
            n => fib(n - 1) + fib(n - 2),
        }
    }

    test_from_file(0, "src/filetests/wasmtime/fib.clif", |out: i32| {
        out == fib(0)
    });

    test_from_file(3, "src/filetests/wasmtime/fib.clif", |out: i32| {
        out == fib(3)
    });

    // todo: higher numbers are currently failing
}
}

#[test]
fn test_basic_exprs() {
    test_from_file((), "src/filetests/expr.clif", |out: i32| out == 12);
}

#[test]
fn test_basic_loop() {
    test_from_file(13, "src/filetests/loop.clif", |res: i32| -> bool {
        res == 0
    })
}

mod ops {
    mod arithmetic {
        use crate::tests::test_from_file;

        #[test]
        fn test_iadd_i32() {
            test_from_file(
                (12, 13),
                "src/filetests/wasmtime/iadd_i32.clif",
                |res: i32| -> bool { res == 12 + 13 },
            );
        }

        #[test]
        fn test_isub_i32() {
            test_from_file(
                (13, 13),
                "src/filetests/wasmtime/isub_i32.clif",
                |res: i32| -> bool { res == 13 - 13 },
            );

            test_from_file(
                (15, 13),
                "src/filetests/wasmtime/isub_i32.clif",
                |res: i32| -> bool { res == 15 - 13 },
            );

            test_from_file(
                (i32::MAX, i32::MAX - i32::MAX / 2),
                "src/filetests/wasmtime/isub_i32.clif",
                |res: i32| -> bool { res == i32::MAX - (i32::MAX - i32::MAX / 2) },
            );
        }
    }

    mod comparisons {
        use crate::tests::test_from_file;

        #[test]
        fn test_i32_eq() {
            test_from_file(
                (12, 12),
                "src/filetests/wasmtime/icmp/eq/i32.clif",
                |res: i32| -> bool { res == 1 },
            );
            test_from_file(
                (12, 13),
                "src/filetests/wasmtime/icmp/eq/i32.clif",
                |res: i32| -> bool { res == 0 },
            );
        }
    }
}

mod control_flow {
    use super::{enable_log, test_from_file};

    #[test]
    fn test_brnz() {
        test_from_file(1, "src/filetests/wasmtime/brnz.clif", |res: i32| -> bool {
            res == 1
        });
        test_from_file(0, "src/filetests/wasmtime/brnz.clif", |res: i32| -> bool {
            res == 2
        });
    }

    #[test]
    fn test_cond_br() {
        enable_log("test_cond_br");
        test_from_file(
            (1, 1),
            "src/filetests/wasmtime/condbr-i32.clif",
            |res: i32| -> bool { res == 1 },
        );
        test_from_file(
            (1, 2),
            "src/filetests/wasmtime/condbr-i32.clif",
            |res: i32| -> bool { res == 2 },
        );
    }
}
