use cranelift_codegen::{
    cursor::FuncCursor,
    ir::{self, Inst, InstructionData},
};
use fnv::FnvHashMap;
use relooper::BranchMode;
use walrus::InstrSeqBuilder;

use super::block::BlockCfCtx;

/// Converts a Cranelift instruction into the corresponding WebAssembly.
///
/// note: this function translates only operations that (a) have a Wasm representation (operations
/// that require a multithreaded environment are not translated) and (b) do not require any control
/// flow (so jumps and branches are handled seperately).
pub fn build_wasm_inst(
    inst: &InstructionData,
    cursor: &mut FuncCursor,
    builder: &mut InstrSeqBuilder,
    ctx: &mut BlockCfCtx<'_>,
) {
    match inst {
        // operations that are unsupportable on WebAssembly
        ir::InstructionData::AtomicCas { .. } | ir::InstructionData::AtomicRmw { .. } => {
            panic!("this operation is not supported on WebAssembly")
        }
        ir::InstructionData::Jump {
            opcode: _,
            args: _,
            destination,
        } => {
            // work out if we can jump to this item
            let mode = if let Some(mode) = ctx.can_branch_to.get(&destination.as_u32()) {
                mode
            } else {
                // (hopefully)
                // todo: fuzzing with Fuzzcheck
                unreachable!()
            };

            match mode {
                BranchMode::LoopBreak(_) => builder.br(todo!()),
                BranchMode::LoopBreakIntoMulti(_)
                | BranchMode::LoopContinue(_)
                | BranchMode::LoopContinueIntoMulti(_)
                | BranchMode::MergedBranch
                | BranchMode::MergedBranchIntoMulti
                | BranchMode::SetLabelAndBreak => todo!(),
            };
        }
        // control flow operations
        // operations that have not yet been implemented
        ir::InstructionData::Binary { .. }
        | ir::InstructionData::BinaryImm64 { .. }
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
    }
}
