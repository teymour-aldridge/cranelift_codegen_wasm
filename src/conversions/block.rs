use cranelift_codegen::{
    cursor::{Cursor, FuncCursor},
    ir::{self, Block, InstructionData},
};
use fnv::FnvHashMap;
use relooper::BranchMode;
use walrus::{ir::InstrSeqId, InstrSeqBuilder};

use super::inst::build_wasm_inst;

/// Useful context about the computed control flow of the instruction in question.
///
/// note: `'clif` is the lifetime of the data structures from Cranelift (`clif` is an abbreviation
/// for Cranelift).
pub struct BlockCfCtx<'clif> {
    /// All the possible positions to which this block may branch.
    pub(crate) can_branch_to: &'clif FnvHashMap<u32, BranchMode>,
    /// Stores which [walrus::ir::InstrSeqId] corresponds to which [cranelift_codegen::ir::Block].
    pub(crate) block_to_seq: &'clif mut FnvHashMap<Block, InstrSeqId>,
    // CRANELIFT <-> RELOOPER INTERFACE
    /// Stores which loops correspond to which Walrus [walrus::ir::InstrSeqId]s.
    pub(crate) loop_to_block: &'clif mut FnvHashMap<u16, InstrSeqId>,
    /// Stores which `match`-style relooper ids correspond to which Walrus
    /// [walrus::ir::InstrSeqId]s
    pub(crate) multi_to_block: &'clif mut FnvHashMap<u16, InstrSeqId>,
}

impl<'clif> BlockCfCtx<'clif> {
    pub fn new(
        can_branch_to: &'clif FnvHashMap<u32, BranchMode>,
        block_to_seq: &'clif mut FnvHashMap<Block, InstrSeqId>,
        loop_to_block: &'clif mut FnvHashMap<u16, InstrSeqId>,
        multi_to_block: &'clif mut FnvHashMap<u16, InstrSeqId>,
    ) -> Self {
        Self {
            can_branch_to,
            block_to_seq,
            loop_to_block,
            multi_to_block,
        }
    }
}

/// Maps Cranlift [cranelift_codegen::ir::Block]s to [walrus::ir::InstrSeq]s.
pub(crate) fn build_wasm_block<'clif>(
    block: ir::Block,
    cursor: &mut FuncCursor,
    builder: &mut InstrSeqBuilder,
    ctx: &mut BlockCfCtx<'clif>,
) {
    cursor.goto_top(block);
    while let Some(next) = cursor.next_inst() {
        match &cursor.func.dfg[next] {
            // we handle control-flow related operations here
            InstructionData::Jump {
                opcode,
                args,
                destination,
            } => {
                // todo: fix this
                let mode = ctx
                    .can_branch_to
                    .get(&destination.as_u32())
                    .expect("internal error - cannot branch to this block");

                match mode {
                    BranchMode::LoopBreak(id) => {
                        // not sure this is actually the correct place to be breaking to
                        // it does seem right though (based on my reading of the relooper source
                        // code)
                        // todo: test this more extensively
                        let seq_id = ctx.loop_to_block.get(id).expect("internal error");
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
            // todo: handle some branches
            // everything else is handled by `build_wasm_inst`
            data => build_wasm_inst(&data.clone(), cursor, builder, ctx),
        }
    }
}
