use cranelift_codegen::ir::{self, Inst, InstInserterBase};
use walrus::{ir::BinaryOp, InstrSeqBuilder};

use crate::{
    conversions::{cond::wasm_of_cond, ty::wasm_of_cranelift},
    IndividualFunctionTranslator, Operand,
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
    match &t.cursor.func.dfg[inst].clone() {
        // operations that are unsupportable on WebAssembly
        ir::InstructionData::AtomicCas { .. } | ir::InstructionData::AtomicRmw { .. } => {
            panic!("this operation is not supported on WebAssembly")
        }
        ir::InstructionData::Binary { opcode, args } => {
            for operand in args {
                translate_value(*operand, t, builder, can_branch_to, inst);
            }
            match opcode {
                ir::Opcode::Iadd => {
                    let [left, _] = args;
                    let ty = t.cursor.data_flow_graph().value_type(*left);
                    if ty == ir::types::I32 {
                        builder.binop(BinaryOp::I32Add);
                    } else if ty == ir::types::I64 {
                        builder.binop(BinaryOp::I64Add);
                    } else {
                        // todo: it's not unreachable yet!
                        unreachable!()
                    }
                }
                ir::Opcode::Isub => {
                    let [left, _] = args;
                    let ty = t.cursor.data_flow_graph().value_type(*left);
                    if ty == ir::types::I32 {
                        builder.binop(BinaryOp::I32Sub);
                    } else if ty == ir::types::I64 {
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
                        return;
                    } else if ty.bits() == 32 {
                        builder.i32_const(imm.bits() as i32);
                        return;
                    }
                } else {
                    panic!()
                }
            } else {
                panic!("this operation is not yet supported")
            }
        }
        ir::InstructionData::MultiAry { opcode, args } => {
            if opcode == &ir::Opcode::Return {
                let pool = &t.cursor.data_flow_graph().value_lists;
                let args = args.as_slice(pool).iter().map(|x| *x).collect::<Vec<_>>();
                for arg in args {
                    translate_value(arg, t, builder, can_branch_to, inst);
                }
                builder.return_();
            }
        }
        ir::InstructionData::IntCompare { opcode, args, cond } => {
            for arg in args {
                translate_value(*arg, t, builder, can_branch_to, inst);
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
                if ty.bits() == 64 {
                    builder.i64_const(imm.bits());
                } else if ty.bits() == 32 {
                    builder.i32_const(imm.bits() as i32);
                } else {
                    unimplemented!()
                }
                translate_value(*arg, t, builder, can_branch_to, inst);
                builder.binop(wasm_of_cond(*cond, imm.bits() == 32));
            } else {
                panic!("{:#?} not yet supported", opcode);
            }
        }
        ir::InstructionData::Jump { .. } | ir::InstructionData::Branch { .. } => {
            unreachable!("this operation should already have been handled")
        }
        // operations that have not yet been implemented
        sth => {
            panic!("support for {:#?} has not yet been implemented", sth)
        }
    }
}
pub(crate) fn translate_value(
    operand: ir::Value,
    t: &mut IndividualFunctionTranslator<'_>,
    builder: &mut InstrSeqBuilder,
    can_branch_to: &CanBranchTo,
    current_inst: Inst,
) {
    match t.cursor.data_flow_graph().value_def(operand) {
        ir::ValueDef::Result(_, _) => {
            match Operand::from_table(operand, &t.operand_table) {
                // it should already have been pushed onto the stack where it was defined
                Operand::SingleUse(_) => {}
                Operand::NormalUse(val) => {
                    if let Some(local) = t.locals.get(&val) {
                        builder.local_get(*local);
                    } else {
                        let def = t.cursor.data_flow_graph().value_def(val).unwrap_inst();
                        build_wasm_inst(def, t, builder, can_branch_to);

                        let arg = t.module_locals.add({
                            let ty = t.cursor.data_flow_graph().value_type(val);
                            wasm_of_cranelift(ty)
                        });

                        t.locals.insert(val, arg);
                        builder.local_set(arg);
                        builder.local_get(arg);
                    }
                }
                Operand::Rematerialise(val) => {
                    let def = t.cursor.data_flow_graph().value_def(val).unwrap_inst();
                    if def != current_inst {
                        build_wasm_inst(def, t, builder, can_branch_to);
                    }
                }
            }
        }
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
