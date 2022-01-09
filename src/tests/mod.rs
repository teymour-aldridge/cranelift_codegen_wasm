use cranelift_codegen::ir;
use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::AbiParam;
use cranelift_codegen::ir::InstBuilder;
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::Variable;
use rusty_fork::rusty_fork_test;

use crate::tests::utils::enable_log;
use crate::tests::utils::run_test;

use self::utils::test_from_file;

mod utils;

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

        // test_from_file(15, "src/filetests/wasmtime/fib.clif", |out: i32| {
        //     out == fib(15)
        // });
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
    use rusty_fork::rusty_fork_test;

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
    fn test_brz() {
        test_from_file(0, "src/filetests/wasmtime/brz.clif", |res: i32| -> bool {
            res == 1
        });
        test_from_file(1, "src/filetests/wasmtime/brz.clif", |res: i32| -> bool {
            res == 0
        });
    }

    #[test]
    fn test_fallthrough() {
        test_from_file(
            42,
            "src/filetests/wasmtime/control.clif",
            |res: i32| -> bool { res == 42 },
        );
    }

    rusty_fork_test! {
        #[test]
        fn test_loop_2() {
            enable_log("test_loop_2");
            test_from_file(1, "src/filetests/loop2.clif", |res: i32| -> bool {
                res == 100
            });
            test_from_file(15, "src/filetests/loop2.clif", |res: i32| -> bool {
                res == 1
            });
        }
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

    rusty_fork_test! {
        #[test]
        fn test_control_flow() {
            enable_log("test_control_flow");
            test_from_file(
                14,
                "src/filetests/control-flow.clif",
                |res: i32| {
                    res == 0
                }
            );
        }
    }
}
