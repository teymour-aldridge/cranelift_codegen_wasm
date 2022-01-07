use cranelift_codegen::{
    cursor::Cursor,
    ir::{self, InstInserterBase, InstructionData},
};
use fnv::FnvHashMap;
use relooper::BranchMode;
use walrus::{ir::BinaryOp, InstrSeqBuilder, LocalId};

use crate::IndividualFunctionTranslator;

use super::inst::translate_value;

pub struct CanBranchTo<'a> {
    pub(crate) from_relooper: &'a FnvHashMap<u32, BranchMode>,
    pub(crate) locally_computed: FnvHashMap<u32, BranchInstr>,
}

#[derive(Debug)]
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
    log::trace!("building block {:?}", block);
    t.cursor.goto_top(block);
    build_from_pos(t, builder, can_branch_to);
}

fn build_from_pos(
    t: &mut IndividualFunctionTranslator,
    builder: &mut InstrSeqBuilder,
    can_branch_to: &CanBranchTo,
) {
    while let Some(next) = t.cursor.next_inst() {
        log::trace!("building instruction: {:?}", next);

        match &t.cursor.func.dfg[next].clone() {
            // we handle control-flow related operations here
            InstructionData::Jump {
                opcode: _,
                args,
                destination,
            } => {
                log::trace!("instruction {:#?} was a jump", next);
                if let Some(jump_to) = t.operand_table.block_params.get(destination) {
                    log::trace!(
                        "obtained table of cranelift<->wasm correspondence: {:?}",
                        jump_to
                    );
                    let args = args
                        .as_slice(&t.cursor.func.dfg.value_lists)
                        .iter()
                        .map(|x| *x)
                        .clone()
                        .collect::<Vec<_>>();
                    log::trace!("args: {:#?}", args);

                    for (value, (_, local)) in args.iter().zip(jump_to.iter()) {
                        translate_value(*value, t, builder, can_branch_to);
                        builder.local_set(*local);
                    }
                } else {
                    log::trace!(
                        "could not find table of arguments for block {:?}",
                        destination
                    );
                }

                if let Some(method) = can_branch_to.locally_computed.get(&destination.as_u32()) {
                    log::trace!("found computed branching method: {:#?}", method);
                    match method {
                        BranchInstr::SetLocal(local) => {
                            builder
                                .i32_const(destination.as_u32() as i32)
                                .local_set(*local);
                        }
                    }
                } else {
                    if let Some(mode) = can_branch_to.from_relooper.get(&destination.as_u32()) {
                        log::trace!("found mode: {:#?}", mode);

                        match mode {
                            // todo: `LoopBreak` and `LoopContinue` are _not_ the same
                            BranchMode::LoopContinue(id) => {
                                // jump back to the top of the loop
                                let seq_id = t.loop_to_block.get(&id).expect("internal error");
                                builder.br(*seq_id);
                            }
                            BranchMode::LoopBreak(_) => {
                                // to break from a loop, we simply don't build any of the remaining
                                // blocks – this means we will reach the end, and then exit
                                return;
                            }
                            // todo: handle these
                            BranchMode::LoopBreakIntoMulti(_) => todo!(),
                            BranchMode::LoopContinueIntoMulti(_) => todo!(),
                            BranchMode::MergedBranch
                            | BranchMode::MergedBranchIntoMulti
                            | BranchMode::SetLabelAndBreak => todo!(),
                        }
                    } else {
                        log::trace!("could not find branching mode from relooper");
                        // todo: is doing nothing the correct way of handling this?
                    };
                }
            }
            ir::InstructionData::MultiAry { opcode, args } => {
                if opcode == &ir::Opcode::Return {
                    let pool = &t.cursor.data_flow_graph().value_lists;
                    let args = args.as_slice(pool).iter().map(|x| *x).collect::<Vec<_>>();
                    log::trace!("args: {:#?}", args);
                    for arg in args {
                        translate_value(arg, t, builder, can_branch_to);
                    }
                    builder.return_();
                } else {
                    panic!("MultiAry {:#?} has not been implemented", opcode)
                }
            }
            ir::InstructionData::Branch {
                opcode,
                args,
                destination,
            } => {
                log::trace!("instruction {:#?} was a branch", next);
                // note: this is needed because `br_if` will continue the loop if true
                // we want to branch based on the truth/false-ness of the operand, so we `br_if`
                // (i.e.) don't branch IF the condition is false, otherwise we do branch
                let negate = can_branch_to
                    .from_relooper
                    .get(&destination.as_u32())
                    .map(|x| match x {
                        BranchMode::LoopBreak(_) => true,
                        _ => false,
                    })
                    .unwrap_or(false);

                log::trace!("negating: {}", negate);

                if let Some(jump_to) = t.operand_table.block_params.get(destination) {
                    log::trace!(
                        "obtained table of cranelift<->wasm correspondence: {:?}",
                        jump_to
                    );
                    let args = args
                        .as_slice(&t.cursor.func.dfg.value_lists)
                        .iter()
                        .map(|x| *x)
                        .clone()
                        .collect::<Vec<_>>();
                    log::trace!("args: {:#?}", args);

                    for (value, (_, local)) in args[1..].iter().zip(jump_to.iter()) {
                        translate_value(*value, t, builder, can_branch_to);
                        builder.local_set(*local);
                    }
                }

                let arg = args.as_slice(&t.cursor.func.dfg.value_lists)[0];
                let ty = t.cursor.data_flow_graph().value_type(arg);

                // if we need to, we negate
                if negate {
                    if ty.bits() == 64 {
                        builder.i64_const(0);
                    } else if ty.bits() <= 32 {
                        builder.i32_const(0);
                    }
                }

                // then we compute the condition
                translate_value(arg, t, builder, can_branch_to);

                // if we need to, we finish the negation
                if negate {
                    if ty.bits() == 64 {
                        builder.binop(BinaryOp::I64Sub);
                    } else {
                        builder.binop(BinaryOp::I32Sub);
                    }
                }

                if opcode == &ir::Opcode::Brz {
                    if ty.is_bool() {
                    } else if ty.bits() == 64 {
                        builder.i64_const(0);
                        builder.binop(BinaryOp::I64Eq);
                    } else if ty.bits() <= 32
                    /* less than or equal because we could have a boolean */
                    {
                        builder.i32_const(0);
                        builder.binop(BinaryOp::I32Eq);
                    } else {
                        unreachable!();
                    };
                }

                if opcode == &ir::Opcode::Brnz {
                    let ty = t.cursor.data_flow_graph().value_type(arg);
                    if ty.is_bool() {
                    } else if ty.bits() == 64 {
                        builder.i64_const(0);
                        builder.binop(BinaryOp::I64Ne);
                    } else if ty.bits() <= 32
                    /* less than or equal because we could have a boolean */
                    {
                        builder.i32_const(0);
                        builder.binop(BinaryOp::I32Ne);
                    } else {
                        unreachable!();
                    };
                }

                // now work out how we are supposed to branch to the next instruction and apply it
                if let Some(mode) = can_branch_to.from_relooper.get(&destination.as_u32()) {
                    match mode {
                        BranchMode::LoopBreak(_) => {
                            builder.if_else(
                                None,
                                |_then| {
                                    // don't do anything, so that if we hit this branch, the loop will
                                    // then immediately exit
                                },
                                |alt| {
                                    build_from_pos(t, alt, can_branch_to);
                                },
                            );
                        }
                        BranchMode::LoopBreakIntoMulti(_) => todo!(),
                        BranchMode::LoopContinue(_) => todo!(),
                        BranchMode::LoopContinueIntoMulti(_) => todo!(),
                        BranchMode::MergedBranchIntoMulti => todo!(),
                        BranchMode::SetLabelAndBreak => todo!(),
                        BranchMode::MergedBranch => {
                            let (_, label) = can_branch_to.locally_computed.iter().next().unwrap();
                            let local = match label {
                                BranchInstr::SetLocal(l) => l,
                            };
                            builder.if_else(
                                None,
                                |then| {
                                    then.i32_const(i32::MAX).local_set(*local);
                                },
                                |alt| {
                                    build_from_pos(t, alt, can_branch_to);
                                },
                            );
                        }
                    }
                } else if let Some(method) =
                    can_branch_to.locally_computed.get(&destination.as_u32())
                {
                    // otherwise, try switching into a multiple block

                    // we computed this earlier
                    if opcode == &ir::Opcode::Brz || opcode == &ir::Opcode::Brnz {
                        match method {
                            BranchInstr::SetLocal(label) => {
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
                        panic!("operation {:#?} not yet supported", opcode)
                    }
                }
            }
            // everything else is handled by `build_wasm_inst`
            sth => {
                log::trace!("skipping {:#?}", sth);
            }
        }
    }
}
