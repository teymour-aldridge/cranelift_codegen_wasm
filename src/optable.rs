use cranelift_codegen::{
    cursor::{Cursor, FuncCursor},
    ir::{self, Block, InstInserterBase},
};
use fnv::{FnvHashMap, FnvHashSet};
use walrus::{LocalId, ModuleLocals};

use crate::conversions::ty::wasm_of_cranelift;

/// Describes the nature of the operand in question.
///
/// Thanks to Chris Fallin for the suggestion
/// https://github.com/bytecodealliance/wasmtime/issues/2566#issuecomment-1003604703
pub(crate) enum Operand {
    /// We are the only use of the operator (so we can just push this onto the
    /// stack).
    SingleUse(ir::Value),
    /// We are _not_ the only use of the operator, so we generate this in a
    /// local at its original location (and we then use the local).
    ///
    /// The [cranelift_codegen::ir::Inst] is the instruction where this function
    /// is defined.
    NormalUse(ir::Value),
    /// Even though the value might be used multiple times, we never store it in
    /// a local (e.g. for operators such as `<ty>.const sth`).
    Rematerialise(ir::Value),
}

impl Operand {
    /// Retrieves the type of the operand from the provided table.
    pub(crate) fn from_table<'ctx>(value: ir::Value, table: &OperandTable) -> Self {
        Operand::try_from_table(value, table).unwrap()
    }

    fn try_from_table(value: ir::Value, table: &OperandTable) -> Option<Self> {
        if table.rematerialize.contains(&value) {
            return Some(Self::Rematerialise(value));
        }

        let val = if let Some(t) = table.value_uses.get(&value) {
            *t
        } else {
            return None;
        };

        Some(if val == 0 || val == 1 {
            Self::SingleUse(value)
        } else {
            Self::NormalUse(value)
        })
    }
}

#[derive(Debug)]
pub struct OperandTable {
    /// Counts the number of times a `[cranelift_codegen::ir::Value]` was used.
    pub(crate) value_uses: FnvHashMap<ir::Value, usize>,
    /// Values which should always be rematerialised.
    pub(crate) rematerialize: FnvHashSet<ir::Value>,
    /// Values which are passed as parameters to a block.
    pub(crate) block_params: FnvHashMap<Block, FnvHashMap<ir::Value, LocalId>>,
}

impl OperandTable {
    /// Computes the role of every [cranelift_codegen::ir::Value] in the
    /// provided program, and adds it to this table.
    pub(crate) fn fill(cursor: &mut FuncCursor, module: &mut ModuleLocals) -> OperandTable {
        let mut value_uses: FnvHashMap<_, _> = Default::default();
        let mut rematerialize: FnvHashSet<_> = Default::default();
        let mut block_params: FnvHashMap<_, _> = Default::default();

        let params = cursor
            .layout()
            .blocks()
            .map(|block| cursor.layout().block_insts(block))
            .flatten()
            .map(|inst| (inst, cursor.data_flow_graph().inst_args(inst)))
            .map(|(inst, values)| values.iter().zip(std::iter::repeat(inst)))
            .flatten();

        for (value, _) in params {
            let def = match cursor.data_flow_graph().value_def(*value) {
                ir::ValueDef::Result(inst, _) => inst,
                ir::ValueDef::Param(block, _) => {
                    log::trace!("got an argument for {:#?}", block);
                    let ty = cursor.data_flow_graph().value_type(*value);
                    let ty = wasm_of_cranelift(ty);
                    let local = module.add(ty);
                    block_params
                        .entry(block)
                        .and_modify(|map: &mut FnvHashMap<_, _>| {
                            map.insert(*value, local);
                        })
                        .or_insert({
                            let mut map = FnvHashMap::default();
                            map.insert(*value, local);
                            map
                        });
                    log::trace!(
                        "entries for block now looks like: {:#?}",
                        block_params.get(&block)
                    );
                    continue;
                }
            };

            let def = &cursor.data_flow_graph()[def];
            match def {
                ir::InstructionData::Unary { opcode, arg: _ }
                | ir::InstructionData::UnaryImm { opcode, imm: _ } => match opcode {
                    ir::Opcode::Iconst => {
                        rematerialize.insert(*value);
                        continue;
                    }
                    _ => (),
                },
                _ => (),
            }

            *value_uses.entry(*value).or_insert(0) += 1;
        }

        Self {
            value_uses,
            rematerialize,
            block_params,
        }
    }
}
