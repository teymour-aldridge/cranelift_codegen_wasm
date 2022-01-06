use cranelift_codegen::ir::{self, InstInserterBase};
use walrus::{ir::BinaryOp, InstrSeqBuilder};

use crate::{
    conversions::{cond::wasm_of_cond, ty::wasm_of_cranelift},
    optable::Operand,
    IndividualFunctionTranslator,
};

use super::block::CanBranchTo;

/// Converts a Cranelift instruction into the corresponding WebAssembly.
///
/// note: this function translates only operations that (a) have a Wasm representation (operations
/// that require a multithreaded environment are not translated) and (b) do not require any control
/// flow (so jumps and branches are handled seperately).
pub fn build_wasm_inst(
    inst: ir::Inst,
    t: &mut IndividualFunctionTranslator<'_>,
    builder: &mut InstrSeqBuilder,
    can_branch_to: &CanBranchTo,
) {
    log::trace!("building instruction {:#?}", inst);
    match &t.cursor.func.dfg[inst].clone() {
        // operations that are unsupportable on WebAssembly
        ir::InstructionData::AtomicCas { .. } | ir::InstructionData::AtomicRmw { .. } => {
            panic!("this operation is not supported on WebAssembly")
        }
        ir::InstructionData::Binary { opcode, args } => {
            log::trace!(
                "instruction is a binary operation with code {:#?} and args {:#?}",
                opcode,
                args
            );
            for operand in args {
                log::trace!("translating operand {:#?}", operand);
                translate_value(*operand, t, builder, can_branch_to);
            }
            match opcode {
                ir::Opcode::Iadd => {
                    log::trace!("opcode is `Iadd`");
                    let [left, _] = args;
                    let ty = t.cursor.data_flow_graph().value_type(*left);
                    if ty == ir::types::I32 {
                        log::trace!("found ty to be i32");
                        builder.binop(BinaryOp::I32Add);
                    } else if ty == ir::types::I64 {
                        log::trace!("found ty to be i64");
                        builder.binop(BinaryOp::I64Add);
                    } else {
                        // todo: it's not unreachable yet!
                        unreachable!()
                    }
                }
                ir::Opcode::Isub => {
                    log::trace!("opcode is `Isub`");
                    let [left, _] = args;
                    let ty = t.cursor.data_flow_graph().value_type(*left);
                    if ty == ir::types::I32 {
                        log::trace!("found ty to be i32");
                        builder.binop(BinaryOp::I32Sub);
                    } else if ty == ir::types::I64 {
                        log::trace!("found ty to be i64");
                        builder.binop(BinaryOp::I64Sub);
                    } else {
                        // todo: it's not unreachable yet!
                        unreachable!()
                    }
                }
                sth => panic!("{:#?} is not yet supported", sth),
            }
        }
        ir::InstructionData::UnaryImm { opcode, imm } => {
            if opcode == &ir::Opcode::Iconst {
                if t.cursor.data_flow_graph().has_results(inst) {
                    let val = t.cursor.data_flow_graph().inst_results(inst)[0];
                    let ty = t.cursor.data_flow_graph().value_type(val);
                    assert!(ty.is_int());
                    if ty.bits() == 64 {
                        builder.i64_const(imm.bits());
                        log::trace!("finished compiling instruction");
                        return;
                    } else if ty.bits() == 32 {
                        builder.i32_const(imm.bits() as i32);
                        log::trace!("finished compiling instruction");
                        return;
                    }
                } else {
                    panic!()
                }
            } else {
                panic!("this operation is not yet supported")
            }
        }
        ir::InstructionData::IntCompare { opcode, args, cond } => {
            for arg in args {
                translate_value(*arg, t, builder, can_branch_to);
            }
            let ty = t.cursor.data_flow_graph().value_type(args[0]);
            assert!(ty.is_int());
            if opcode == &ir::Opcode::Icmp {
                match cond {
                    ir::condcodes::IntCC::NotEqual => {
                        if ty.bits() == 64 {
                            builder.binop(BinaryOp::I64Ne);
                        } else if ty.bits() == 32 {
                            builder.binop(BinaryOp::I32Ne);
                        } else {
                            panic!("integers must be 32 or 64 bits")
                        }
                    }
                    ir::condcodes::IntCC::Equal => {
                        if ty.bits() == 64 {
                            builder.binop(BinaryOp::I64Eq);
                        } else if ty.bits() == 32 {
                            builder.binop(BinaryOp::I32Eq);
                        } else {
                            panic!("integers must be 32 or 64 bits")
                        }
                    }
                    ir::condcodes::IntCC::SignedLessThan
                    | ir::condcodes::IntCC::SignedGreaterThanOrEqual
                    | ir::condcodes::IntCC::SignedGreaterThan
                    | ir::condcodes::IntCC::SignedLessThanOrEqual
                    | ir::condcodes::IntCC::UnsignedLessThan
                    | ir::condcodes::IntCC::UnsignedGreaterThanOrEqual
                    | ir::condcodes::IntCC::UnsignedGreaterThan
                    | ir::condcodes::IntCC::UnsignedLessThanOrEqual
                    | ir::condcodes::IntCC::Overflow
                    | ir::condcodes::IntCC::NotOverflow => todo!(),
                }
            } else {
                panic!("operation not yet supported");
            }
        }
        ir::InstructionData::IntCompareImm {
            opcode,
            arg,
            cond,
            imm,
        } => {
            let ty = t.cursor.data_flow_graph().value_type(*arg);
            assert!(ty.is_int());
            if opcode == &ir::Opcode::IcmpImm {
                translate_value(*arg, t, builder, can_branch_to);
                if ty.bits() == 64 {
                    builder.i64_const(imm.bits());
                } else if ty.bits() == 32 {
                    builder.i32_const(imm.bits() as i32);
                } else {
                    unimplemented!()
                }
                builder.binop(wasm_of_cond(*cond, ty.bits() == 32));
            } else {
                panic!("{:#?} not yet supported", opcode);
            }
        }
        ir::InstructionData::Jump { .. }
        | ir::InstructionData::Branch { .. }
        | ir::InstructionData::MultiAry { .. } => {
            unreachable!("this operation should already have been handled")
        }
        ir::InstructionData::BinaryImm64 { opcode, arg, imm } => {
            if opcode == &ir::Opcode::IaddImm {
                let ty = t.cursor.data_flow_graph().value_type(*arg);
                assert!(ty.is_int());
                translate_value(*arg, t, builder, can_branch_to);
                if ty.bits() == 64 {
                    builder.i64_const(imm.bits());
                    builder.binop(BinaryOp::I64Add);
                } else if ty.bits() == 32 {
                    builder.i32_const(imm.bits() as i32);
                    builder.binop(BinaryOp::I32Add);
                } else {
                    unimplemented!();
                }
            }
        }
        // operations that have not yet been implemented
        sth => {
            panic!("support for {:#?} has not yet been implemented", sth)
        }
    }
    log::trace!("finished compiling instruction");
}
pub(crate) fn translate_value(
    operand: ir::Value,
    t: &mut IndividualFunctionTranslator<'_>,
    builder: &mut InstrSeqBuilder,
    can_branch_to: &CanBranchTo,
) {
    match t.cursor.data_flow_graph().value_def(operand) {
        ir::ValueDef::Result(_, _) => match Operand::from_table(operand, &t.operand_table) {
            Operand::SingleUse(val) => {
                let def = t.cursor.data_flow_graph().value_def(val).unwrap_inst();
                build_wasm_inst(def, t, builder, can_branch_to);
            }
            Operand::NormalUse(val) => {
                if let Some(local) = t.locals.get(&val) {
                    log::trace!("{:#?} is `NormalUse` and has previously been used", val);
                    log::trace!("retrieving {:#?} from local {:#?}", val, local);
                    builder.local_get(*local);
                } else {
                    log::trace!("{:#?} is `NormalUse` and has not previously been used", val);
                    let def = t.cursor.data_flow_graph().value_def(val).unwrap_inst();
                    build_wasm_inst(def, t, builder, can_branch_to);

                    let arg = t.module_locals.add({
                        let ty = t.cursor.data_flow_graph().value_type(val);
                        wasm_of_cranelift(ty)
                    });

                    t.locals.insert(val, arg);
                    log::trace!("{:#?} has been assigned to local {:#?}", val, arg);
                    builder.local_set(arg);
                    builder.local_get(arg);
                }
            }
            Operand::Rematerialise(val) => {
                let def = t.cursor.data_flow_graph().value_def(val).unwrap_inst();
                build_wasm_inst(def, t, builder, can_branch_to);
            }
        },
        ir::ValueDef::Param(block, _) => {
            let local = t
                .operand_table
                .block_params
                .get(&block)
                .map(|res| res.get(&operand))
                .flatten()
                .unwrap();
            builder.local_get(*local);
        }
    }
}
