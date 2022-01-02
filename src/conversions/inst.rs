use cranelift_codegen::ir::{self, InstInserterBase, InstructionData};
use fnv::FnvHashMap;
use relooper::BranchMode;
use walrus::{ir::BinaryOp, InstrSeqBuilder};

use crate::{conversions::ty::wasm_of_cranelift, IndividualFunctionTranslator, Operand};

/// Converts a Cranelift instruction into the corresponding WebAssembly.
///
/// note: this function translates only operations that (a) have a Wasm representation (operations
/// that require a multithreaded environment are not translated) and (b) do not require any control
/// flow (so jumps and branches are handled seperately).
pub fn build_wasm_inst(
    inst: InstructionData,
    t: &mut IndividualFunctionTranslator<'_>,
    builder: &mut InstrSeqBuilder,
    can_branch_to: &FnvHashMap<u32, BranchMode>,
) {
    match inst {
        // operations that are unsupportable on WebAssembly
        ir::InstructionData::AtomicCas { .. } | ir::InstructionData::AtomicRmw { .. } => {
            panic!("this operation is not supported on WebAssembly")
        }
        ir::InstructionData::Binary { opcode, args } => {
            for operand in args {
                match Operand::from_table(operand, &t.operand_table) {
                    Operand::SingleUse(val) => {
                        let def = t.cursor.data_flow_graph().value_def(val).unwrap_inst();
                        let def = t.cursor.data_flow_graph()[def].clone();
                        build_wasm_inst(def, t, builder, can_branch_to);
                    }
                    Operand::NormalUse(val) => {
                        if let Some(local) = t.locals.get(&val) {
                            builder.local_get(*local);
                        } else {
                            let def = t.cursor.data_flow_graph().value_def(val).unwrap_inst();
                            let def = t.cursor.data_flow_graph()[def].clone();
                            build_wasm_inst(def, t, builder, can_branch_to);

                            let arg = t.module.locals.add({
                                let ty = t.cursor.data_flow_graph().value_type(val);
                                wasm_of_cranelift(ty)
                            });

                            builder.local_set(arg);
                        }
                    }
                    Operand::Rematerialise(val) => {
                        let def_inst = t.cursor.data_flow_graph().value_def(val).unwrap_inst();
                        let def = t.cursor.data_flow_graph()[def_inst].clone();
                        build_wasm_inst(def, t, builder, can_branch_to);
                    }
                }
            }
            match opcode {
                ir::Opcode::Iadd => {
                    let [left, _] = args;
                    let ty = t.cursor.data_flow_graph().value_type(left);
                    if ty == ir::types::I32 {
                        builder.binop(BinaryOp::I32Add);
                    } else if ty == ir::types::I64 {
                        builder.binop(BinaryOp::I64Add);
                    } else {
                        // todo: it's not unreachable yet!
                        unreachable!()
                    }
                }
                _ => todo!(),
            }
        }
        // operations that have not yet been implemented
        ir::InstructionData::BinaryImm64 { .. }
        | ir::InstructionData::BinaryImm8 { .. }
        | ir::InstructionData::Branch { .. }
        | ir::InstructionData::BranchIcmp { .. }
        | ir::InstructionData::BranchInt { .. }
        | ir::InstructionData::BranchTable { .. }
        | ir::InstructionData::CallIndirect { .. }
        | ir::InstructionData::BranchFloat { .. }
        | ir::InstructionData::Call { .. }
        | ir::InstructionData::CondTrap { .. }
        | ir::InstructionData::FloatCompare { .. }
        | ir::InstructionData::FloatCond { .. }
        | ir::InstructionData::FloatCondTrap { .. }
        | ir::InstructionData::FuncAddr { .. }
        | ir::InstructionData::HeapAddr { .. }
        | ir::InstructionData::IntCompare { .. }
        | ir::InstructionData::IntCompareImm { .. }
        | ir::InstructionData::IntCond { .. }
        | ir::InstructionData::IntCondTrap { .. }
        | ir::InstructionData::IntSelect { .. }
        | ir::InstructionData::Load { .. }
        | ir::InstructionData::LoadComplex { .. }
        | ir::InstructionData::LoadNoOffset { .. }
        | ir::InstructionData::MultiAry { .. }
        | ir::InstructionData::NullAry { .. }
        | ir::InstructionData::Shuffle { .. }
        | ir::InstructionData::StackLoad { .. }
        | ir::InstructionData::StackStore { .. }
        | ir::InstructionData::Store { .. }
        | ir::InstructionData::StoreComplex { .. }
        | ir::InstructionData::StoreNoOffset { .. }
        | ir::InstructionData::TableAddr { .. }
        | ir::InstructionData::Ternary { .. }
        | ir::InstructionData::TernaryImm8 { .. }
        | ir::InstructionData::Trap { .. }
        | ir::InstructionData::Unary { .. }
        | ir::InstructionData::UnaryBool { .. }
        | ir::InstructionData::UnaryConst { .. }
        | ir::InstructionData::UnaryGlobalValue { .. }
        | ir::InstructionData::UnaryIeee32 { .. }
        | ir::InstructionData::UnaryIeee64 { .. }
        | ir::InstructionData::UnaryImm { .. } => {
            panic!("this operation is not yet supported")
        }
        ir::InstructionData::Jump { .. } => {
            unreachable!("this operation should already have been handled")
        }
    }
}
