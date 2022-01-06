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
                if let Some(jump_to) = t.operand_table.block_params.get(destination) {
                    let args = args
                        .as_slice(&t.cursor.func.dfg.value_lists)
                        .iter()
                        .map(|x| *x)
                        .clone()
                        .collect::<Vec<_>>();

                    for (value, (_, local)) in args.iter().zip(jump_to.iter()) {
                        translate_value(*value, t, builder, can_branch_to);
                        builder.local_set(*local);
                    }
                }

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
            ir::InstructionData::MultiAry { opcode, args } => {
                if opcode == &ir::Opcode::Return {
                    let pool = &t.cursor.data_flow_graph().value_lists;
                    let args = args.as_slice(pool).iter().map(|x| *x).collect::<Vec<_>>();
                    for arg in args {
                        translate_value(arg, t, builder, can_branch_to);
                    }
                    builder.return_();
                }
            }
            ir::InstructionData::Branch {
                opcode,
                args,
                destination,
            } => {
                if let Some(jump_to) = t.operand_table.block_params.get(destination) {
                    let args = args
                        .as_slice(&t.cursor.func.dfg.value_lists)
                        .iter()
                        .map(|x| *x)
                        .clone()
                        .collect::<Vec<_>>();
                    for (value, (_, local)) in args[1..].iter().zip(jump_to.iter()) {
                        translate_value(*value, t, builder, can_branch_to);
                        builder.local_set(*local);
                    }
                }

                // first we compute the condition
                let arg = args.as_slice(&t.cursor.func.dfg.value_lists)[0];
                translate_value(arg, t, builder, can_branch_to);

                if opcode == &ir::Opcode::Brz {
                    let ty = t.cursor.data_flow_graph().value_type(arg);
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
                        BranchMode::LoopBreak(id) => {
                            let seq_id = t.loop_to_block.get(id).unwrap();
                            builder.br_if(*seq_id);
                            return;
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
                            return;
                        }
                    }
                }

                // otherwise, try switching into a multiple block

                // we computed this earlier
                let method = can_branch_to
                    .locally_computed
                    .get(&destination.as_u32())
                    .unwrap();
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
            // everything else is handled by `build_wasm_inst`
            _ => (),
        }
    }
}
