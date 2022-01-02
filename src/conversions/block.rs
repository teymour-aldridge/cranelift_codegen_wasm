use cranelift_codegen::{
    cursor::Cursor,
    ir::{self, InstructionData},
};
use fnv::FnvHashMap;
use relooper::BranchMode;
use walrus::InstrSeqBuilder;

use crate::IndividualFunctionTranslator;

use super::inst::build_wasm_inst;

/// Maps Cranlift [cranelift_codegen::ir::Block]s to [walrus::ir::InstrSeq]s.
pub(crate) fn build_wasm_block<'clif>(
    block: ir::Block,
    t: &mut IndividualFunctionTranslator<'_>,
    builder: &mut InstrSeqBuilder,
    can_branch_to: &FnvHashMap<u32, BranchMode>,
) {
    t.cursor.goto_top(block);
    while let Some(next) = t.cursor.next_inst() {
        match &t.cursor.func.dfg[next] {
            // we handle control-flow related operations here
            InstructionData::Jump {
                opcode: _,
                // todo: handle these!
                args: _,
                destination,
            } => {
                // todo: fix this
                let mode = can_branch_to
                    .get(&destination.as_u32())
                    .expect("internal error - cannot branch to this block");

                match mode {
                    BranchMode::LoopBreak(id) => {
                        // not sure this is actually the correct place to be breaking to
                        // it does seem right though (based on my reading of the relooper source
                        // code)
                        // todo: test this more extensively
                        let seq_id = t.loop_to_block.get(id).expect("internal error");
                        builder.br(*seq_id);
                    }
                    BranchMode::LoopBreakIntoMulti(_) => todo!(),
                    BranchMode::LoopContinue(_) => todo!(),
                    BranchMode::LoopContinueIntoMulti(_) => todo!(),
                    BranchMode::MergedBranch
                    | BranchMode::MergedBranchIntoMulti
                    | BranchMode::SetLabelAndBreak => todo!(),
                }
            }
            // todo: handle some other control-flow operations
            // everything else is handled by `build_wasm_inst`
            data => build_wasm_inst(data.clone(), t, builder, can_branch_to),
        }
    }
}
