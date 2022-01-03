//! A WebAssembly module for Cranelift.

mod conversions;

use cranelift_codegen::{
    binemit,
    cursor::{Cursor, FuncCursor},
    ir::{self, instructions::BranchInfo, Block, InstInserterBase},
    isa::TargetIsa,
    Context,
};
use cranelift_module::{
    DataContext, DataId, FuncId, Linkage, Module as CraneliftModule, ModuleCompiledFunction,
    ModuleDeclarations, ModuleResult,
};
use fnv::{FnvHashMap, FnvHashSet};
use relooper::{reloop, ShapedBlock};
use walrus::{
    ir::{BinaryOp, InstrSeqId},
    DataKind, FunctionBuilder, InstrSeqBuilder, LocalFunction, LocalId, MemoryId,
    Module as WalrusModule, ModuleConfig, ValType,
};

use crate::conversions::{block::build_wasm_block, sig::wasm_of_sig};

/// A WebAssembly module.
pub struct WasmModule {
    /// data we are receiving from Cranelift
    decls: ModuleDeclarations,
    /// data we are creating with Walrus.
    module: WalrusModule,
    /// configuration from Cranelift.
    #[allow(unused)]
    config: ModuleConfig,
    #[allow(unused)]
    memory_id: MemoryId,
    isa: Box<dyn TargetIsa>,
    /// Maps Cranelift functions to Walrus functions.
    functions: FnvHashMap<FuncId, walrus::FunctionId>,
    anon_func_number: usize,
    /// Maps Cranelift data items to Walrus data items.
    data: FnvHashMap<DataId, walrus::DataId>,
    anon_data_number: usize,
}

impl WasmModule {
    /// Constructs a new WebAssembly module.
    ///
    /// todo: check the target isa (or maybe don't take it as a parameter, and generate it instead)
    pub fn new(config: ModuleConfig, isa: Box<dyn TargetIsa>) -> Self {
        let mut module = WalrusModule::default();

        let memory_id = module.memories.add_local(false, 1000, None);

        Self {
            decls: Default::default(),
            module,
            config,
            memory_id,
            isa,
            anon_data_number: 0,
            functions: Default::default(),
            data: Default::default(),
            anon_func_number: 0,
        }
    }
}

impl CraneliftModule for WasmModule {
    fn isa(&self) -> &dyn TargetIsa {
        &*self.isa
    }

    fn declarations(&self) -> &ModuleDeclarations {
        &self.decls
    }

    fn declare_function(
        &mut self,
        name: &str,
        linkage: Linkage,
        signature: &ir::Signature,
    ) -> ModuleResult<FuncId> {
        self.decls
            .declare_function(name, linkage, signature)
            .map(|(a, _)| a)
    }

    fn declare_anonymous_function(&mut self, signature: &ir::Signature) -> ModuleResult<FuncId> {
        self.decls.declare_anonymous_function(signature)
    }

    fn declare_data(
        &mut self,
        _name: &str,
        _linkage: Linkage,
        writable: bool,
        tls: bool,
    ) -> ModuleResult<DataId> {
        self.decls.declare_anonymous_data(writable, tls)
    }

    fn declare_anonymous_data(&mut self, writable: bool, tls: bool) -> ModuleResult<DataId> {
        self.anon_data_number += 1;
        let clif_data_id = self.decls.declare_anonymous_data(writable, tls)?;
        let walrus_data_id = self.module.data.add(DataKind::Passive, Vec::new());
        self.data.insert(clif_data_id, walrus_data_id);
        Ok(clif_data_id)
    }

    fn define_function(
        &mut self,
        _func: FuncId,
        ctx: &mut Context,
        _trap_sink: &mut dyn binemit::TrapSink,
        _stack_map_sink: &mut dyn binemit::StackMapSink,
    ) -> ModuleResult<ModuleCompiledFunction> {
        // set up WebAssembly function
        let (params, returns) = wasm_of_sig(ctx.func.signature.clone());
        let mut wasm_func = FunctionBuilder::new(&mut self.module.types, &params, &returns);
        let mut body = wasm_func.func_body();

        // set up Cranelift
        let mut cursor = FuncCursor::new(&mut ctx.func);

        let operand_table = OperandTable::fill(&mut cursor);

        // todo: check if function is empty!
        let blocks: Vec<_> = cursor
            .func
            .layout
            .blocks()
            .map(|block| block.clone())
            .collect();

        // note: the relooper crate does not have much documentation, but the original Emscripten
        // paper explains it quite well: https://dl.acm.org/doi/10.1145/2048147.2048224
        // also available at https://github.com/emscripten-core/emscripten/blob/main/docs/paper.pdf
        let mut relooper_blocks = Vec::new();

        for block in &blocks {
            let mut branches = vec![];
            cursor.goto_top(*block);

            while let Some(inst) = cursor.next_inst() {
                match cursor.func.dfg.analyze_branch(inst) {
                    BranchInfo::NotABranch => (),
                    BranchInfo::SingleDest(block, _) => {
                        branches.push(block.as_u32());
                    }
                    BranchInfo::Table(_, _) => todo!(),
                }
            }

            relooper_blocks.push((block.as_u32(), branches))
        }

        let first = blocks.first().map(|b| b.as_u32()).unwrap();

        let structured = reloop(relooper_blocks, first);

        let (mut block_to_seq, mut loop_to_block, mut multi_to_block) =
            (Default::default(), Default::default(), Default::default());

        let mut locals = Default::default();

        let mut translator = IndividualFunctionTranslator::new(
            &mut self.module,
            &mut cursor,
            &mut block_to_seq,
            &mut loop_to_block,
            &mut multi_to_block,
            &operand_table,
            &mut locals,
        );

        translator.compile_structured(&mut body, &structured);

        Ok(ModuleCompiledFunction {
            // todo: compute size correctly
            size: 0,
        })
    }

    fn define_function_bytes(
        &mut self,
        _func: FuncId,
        _bytes: &[u8],
        _relocs: &[cranelift_module::RelocRecord],
    ) -> ModuleResult<ModuleCompiledFunction> {
        todo!()
    }

    fn define_data(&mut self, data: DataId, data_ctx: &DataContext) -> ModuleResult<()> {
        let walrus_id = self.data.get(&data).unwrap();
        let data = self.module.data.get_mut(*walrus_id);

        let desc = data_ctx.description();

        match &desc.init {
            cranelift_module::Init::Uninitialized => todo!(),
            cranelift_module::Init::Zeros { size } => {
                data.value = vec![0; *size];
            }
            cranelift_module::Init::Bytes { contents } => data.value = contents.to_vec(),
        }

        Ok(())
    }
}

/// Houses all the data structures needed to compile a function.
///
/// todo: sort out visiblity rules
pub struct IndividualFunctionTranslator<'clif> {
    /// The Walrus module to which we are emitting WebAssembly.
    module: &'clif mut WalrusModule,
    /// The cursor which we are using to query useful relevant information from Cranelift about the
    /// nature of the IR with which we are being provided.
    cursor: &'clif mut FuncCursor<'clif>,
    // todo: is this one even needed?
    #[allow(unused)]
    block_to_seq: &'clif mut FnvHashMap<Block, InstrSeqId>,
    #[allow(unused)]
    loop_to_block: &'clif mut FnvHashMap<u16, InstrSeqId>,
    #[allow(unused)]
    multi_to_block: &'clif mut FnvHashMap<u16, InstrSeqId>,
    current_label: Option<LocalId>,
    operand_table: &'clif OperandTable,
    locals: &'clif mut FnvHashMap<ir::Value, LocalId>,
}

impl<'clif> IndividualFunctionTranslator<'clif> {
    fn new(
        module: &'clif mut WalrusModule,
        cursor: &'clif mut FuncCursor<'clif>,
        block_to_seq: &'clif mut FnvHashMap<Block, InstrSeqId>,
        loop_to_block: &'clif mut FnvHashMap<u16, InstrSeqId>,
        multi_to_block: &'clif mut FnvHashMap<u16, InstrSeqId>,
        operand_table: &'clif OperandTable,
        locals: &'clif mut FnvHashMap<ir::Value, LocalId>,
    ) -> Self {
        Self {
            module,
            cursor,
            block_to_seq,
            loop_to_block,
            multi_to_block,
            current_label: None,
            operand_table,
            locals,
        }
    }

    fn compile_structured(&mut self, builder: &mut InstrSeqBuilder, structured: &ShapedBlock<u32>) {
        match structured {
            // a straight-line sequence of blocks
            // we just translate each one in turn
            ShapedBlock::Simple(simple) => {
                let can_branch_to = &simple.branches;

                build_wasm_block(Block::from_u32(simple.label), self, builder, &can_branch_to);

                if let Some(ref next) = simple.next {
                    self.compile_structured(builder, next);
                }
            }
            ShapedBlock::Loop(l) => {
                builder.loop_(None, |builder: &mut InstrSeqBuilder| {
                    self.loop_to_block.insert(l.loop_id, builder.id());
                    self.compile_structured(builder, &l.inner)
                });

                if let Some(ref next) = l.next {
                    // todo: provide loop context (for breaking from them)
                    self.compile_structured(builder, next);
                }
            }
            // `match`/`if` + `else if` chain
            ShapedBlock::Multiple(m) => {
                // note: `HandledBlock::break_after` means "can this entry reach another entry"

                // we create a local storing the label
                let label = self.module.locals.add(ValType::I32);
                let current_val = self.current_label.clone();
                self.current_label = Some(label);
                // todo: this might panic – handle that case better
                // then we set the label to the first item
                builder
                    .i32_const(
                        *m.handled
                            .first()
                            .map(|item| item.labels.first())
                            .flatten()
                            .unwrap() as i32,
                    )
                    .local_set(label);

                // now we run the if-else sequence
                // once for each node in the state machine
                for each in &m.handled {
                    // and then once for each possible label
                    for val in &each.labels {
                        builder
                            .local_get(label)
                            .i32_const(*val as i32)
                            .binop(BinaryOp::I32Eq)
                            .if_else(
                                ValType::I32,
                                |builder| {
                                    self.compile_structured(builder, &each.inner);
                                },
                                |_| {},
                            );
                    }
                }

                self.current_label = current_val;
            }
        }
    }
}

/// Describes the nature of the operand in question.
///
/// Thanks to Chris Fallin for the suggestion
/// https://github.com/bytecodealliance/wasmtime/issues/2566#issuecomment-1003604703
enum Operand {
    /// We are the only use of the operator (so we can just push this onto the stack).
    SingleUse(ir::Value),
    /// We are _not_ the only use of the operator, so we generate this in a local at its original
    /// location (and we then use the local).
    ///
    /// The [cranelift_codegen::ir::Inst] is the instruction where this function is defined.
    NormalUse(ir::Value),
    /// Even though the value might be used multiple times, we never store it in a local (e.g. for
    /// operators such as `<ty>.const sth`).
    Rematerialise(ir::Value),
}

impl Operand {
    /// Retrieves the type of the operand from the provided table.
    fn from_table<'ctx>(value: ir::Value, table: &OperandTable) -> Self {
        if table.rematerialize.contains(&value) {
            return Self::Rematerialise(value);
        }

        let val = *table.value_uses.get(&value).unwrap();

        // todo: check for constants

        if val == 0 || val == 1 {
            Self::SingleUse(value)
        } else {
            Self::NormalUse(value)
        }
    }
}

pub struct OperandTable {
    /// Counts the number of times a `[cranelift_codegen::ir::Value]` was used.
    value_uses: FnvHashMap<ir::Value, usize>,
    /// Values which should always be rematerialised.
    rematerialize: FnvHashSet<ir::Value>,
}

impl OperandTable {
    /// Computes the role of every [cranelift_codegen::ir::Value] in the provided program, and adds
    /// it to this table.
    fn fill(cursor: &mut FuncCursor) -> OperandTable {
        let mut value_uses: FnvHashMap<_, _> = Default::default();
        let mut rematerialize: FnvHashSet<_> = Default::default();

        let params = cursor
            .layout()
            .blocks()
            .map(|block| cursor.layout().block_insts(block))
            .flatten()
            .map(|inst| (inst, cursor.data_flow_graph().inst_args(inst)))
            .map(|(inst, values)| values.iter().zip(std::iter::repeat(inst)))
            .flatten();

        for (value, _) in params {
            let def = cursor.data_flow_graph().value_def(*value).unwrap_inst();
            let def = &cursor.data_flow_graph()[def];
            match def {
                ir::InstructionData::Unary { opcode, arg: _ } => match opcode {
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
        }
    }
}
