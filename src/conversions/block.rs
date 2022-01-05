use cranelift_codegen::{
    cursor::Cursor,
    ir::{self, InstructionData},
};
use fnv::FnvHashMap;
use relooper::BranchMode;
use walrus::{InstrSeqBuilder, LocalId};

use crate::IndividualFunctionTranslator;

use super::inst::{build_wasm_inst, translate_value};

pub struct CanBranchTo<'a> {
    pub(crate) from_relooper: &'a FnvHashMap<u32, BranchMode>,
    pub(crate) locally_computed: FnvHashMap<u32, BranchInstr>,
}

pub enum BranchInstr {
    SetLocal(LocalId),
}

/// Maps Cranlift [cranelift_codegen::ir::Block]s to [walrus::ir::InstrSeq]s.
pub(crate) fn build_wasm_block<'clif>(
    block: ir::Block,
    t: &mut IndividualFunctionTranslator<'_>,
    builder: &mut InstrSeqBuilder,
    can_branch_to: &CanBranchTo,
) {
    t.cursor.goto_top(block);
    build_from_pos(t, builder, can_branch_to);
}

fn build_from_pos(
    t: &mut IndividualFunctionTranslator,
    builder: &mut InstrSeqBuilder,
    can_branch_to: &CanBranchTo,
) {
    while let Some(next) = t.cursor.next_inst() {
        match &t.cursor.func.dfg[next].clone() {
            // we handle control-flow related operations here
            InstructionData::Jump {
                opcode: _,
                args,
                destination,
            } => {
                if let Some(method) = can_branch_to.locally_computed.get(&destination.as_u32()) {
                    match method {
                        BranchInstr::SetLocal(local) => {
                            builder
                                .i32_const(destination.as_u32() as i32)
                                .local_set(*local);
                            return;
                        }
                    }
                }

                let mode = if let Some(b) = can_branch_to.from_relooper.get(&destination.as_u32()) {
                    b
                } else {
                    // todo: is this the correct way of handling this?
                    return;
                };

                match mode {
                    BranchMode::LoopBreak(id) | BranchMode::LoopContinue(id) => {
                        // not sure this is actually the correct place to be breaking to
                        // it does seem right though (based on my reading of the relooper source
                        // code)
                        // todo: test this more extensively
                        let jump_to = t.operand_table.block_params.get(destination).unwrap();
                        let args = args
                            .as_slice(&t.cursor.func.dfg.value_lists)
                            .iter()
                            .map(|x| *x)
                            .clone()
                            .collect::<Vec<_>>();
                        // set all the parameters that the destination block requires
                        for (value, (_, local)) in args.iter().zip(jump_to.iter()) {
                            translate_value(*value, t, builder, can_branch_to, next);
                            builder.local_set(*local);
                        }
                        // break to the next block
                        let seq_id = t.loop_to_block.get(&id).expect("internal error");
                        builder.br(*seq_id);
                    }
                    BranchMode::LoopBreakIntoMulti(_) => todo!(),
                    BranchMode::LoopContinueIntoMulti(_) => todo!(),
                    BranchMode::MergedBranch
                    | BranchMode::MergedBranchIntoMulti
                    | BranchMode::SetLabelAndBreak => todo!(),
                }
            }
            ir::InstructionData::Branch {
                opcode,
                args,
                destination,
            } => {
                if opcode == &ir::Opcode::Brz {
                    let arg = args.as_slice(&t.cursor.func.dfg.value_lists)[0];
                    translate_value(arg, t, builder, can_branch_to, next);

                    let method = can_branch_to
                        .locally_computed
                        .get(&destination.as_u32())
                        .unwrap();

                    match method {
                        BranchInstr::SetLocal(label) => {
                            if let Some(jump_to) = t.operand_table.block_params.get(destination) {
                                let args = args
                                    .as_slice(&t.cursor.func.dfg.value_lists)
                                    .iter()
                                    .map(|x| *x)
                                    .clone()
                                    .collect::<Vec<_>>();
                                for (value, (_, local)) in args.iter().zip(jump_to.iter()) {
                                    translate_value(*value, t, builder, can_branch_to, next);
                                    builder.local_set(*local);
                                }
                            }

                            builder.if_else(
                                None,
                                |then| {
                                    then.i32_const(destination.as_u32() as i32)
                                        .local_set(*label);
                                },
                                |alt| {
                                    build_from_pos(t, alt, can_branch_to);
                                },
                            );
                        }
                    }
                } else {
                    panic!("operation not yet supported")
                }
            }
            // we ignore this here, because these are generated when they are later rematerialized
            ir::InstructionData::UnaryImm { .. } => {}
            // todo: handle some other control-flow operations
            // everything else is handled by `build_wasm_inst`
            _ => build_wasm_inst(next, t, builder, can_branch_to),
        }
    }
}
