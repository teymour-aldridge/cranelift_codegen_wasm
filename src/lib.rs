//! A WebAssembly module for Cranelift.

mod conversions;

use cranelift_codegen::{
    binemit,
    cursor::{Cursor, FuncCursor},
    ir::{self, instructions::BranchInfo, Block},
    isa::TargetIsa,
    Context,
};
use cranelift_module::{
    DataContext, DataId, FuncId, Linkage, Module as CraneliftModule, ModuleCompiledFunction,
    ModuleDeclarations, ModuleResult,
};
use fnv::FnvHashMap;
use relooper::{reloop, ShapedBlock};
use walrus::{
    ir::{BinaryOp, InstrSeqId},
    FunctionBuilder, InstrSeqBuilder, LocalId, Module as WalrusModule, ModuleConfig, ValType,
};

use crate::conversions::{
    block::{build_wasm_block, BlockCfCtx},
    sig::wasm_of_sig,
};

/// A WebAssembly module.
struct WasmModule {
    /// data we are receiving from Cranelift
    decls: ModuleDeclarations,
    /// data we are creating with Walrus.
    module: WalrusModule,
    /// configuration from Cranelift.
    #[allow(unused)]
    config: ModuleConfig,
}

impl CraneliftModule for WasmModule {
    fn isa(&self) -> &dyn TargetIsa {
        // unimplemented
        todo!()
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

    fn declare_anonymous_function(&mut self, _signature: &ir::Signature) -> ModuleResult<FuncId> {
        todo!()
    }

    fn declare_data(
        &mut self,
        _name: &str,
        _linkage: Linkage,
        _writable: bool,
        _tls: bool,
    ) -> ModuleResult<DataId> {
        todo!()
    }

    fn declare_anonymous_data(&mut self, _writable: bool, _tls: bool) -> ModuleResult<DataId> {
        todo!()
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

        let mut translator = IndividualFunctionTranslator::new(
            &mut self.module,
            &mut cursor,
            &mut block_to_seq,
            &mut loop_to_block,
            &mut multi_to_block,
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

    fn define_data(&mut self, _data: DataId, _data_ctx: &DataContext) -> ModuleResult<()> {
        todo!()
    }
}

struct IndividualFunctionTranslator<'clif> {
    /// The Walrus module to which we are emitting WebAssembly.
    module: &'clif mut WalrusModule,
    /// The cursor which we are using to query useful relevant information from Cranelift about the
    /// nature of the IR with which we are being provided.
    cursor: &'clif mut FuncCursor<'clif>,
    // todo: is this one even needed?
    block_to_seq: &'clif mut FnvHashMap<Block, InstrSeqId>,
    loop_to_block: &'clif mut FnvHashMap<u16, InstrSeqId>,
    multi_to_block: &'clif mut FnvHashMap<u16, InstrSeqId>,
    current_label: Option<LocalId>,
}

impl<'clif> IndividualFunctionTranslator<'clif> {
    fn new(
        module: &'clif mut WalrusModule,
        cursor: &'clif mut FuncCursor<'clif>,
        block_to_seq: &'clif mut FnvHashMap<Block, InstrSeqId>,
        loop_to_block: &'clif mut FnvHashMap<u16, InstrSeqId>,
        multi_to_block: &'clif mut FnvHashMap<u16, InstrSeqId>,
    ) -> Self {
        Self {
            module,
            cursor,
            block_to_seq,
            loop_to_block,
            multi_to_block,
            current_label: None,
        }
    }

    fn compile_structured(&mut self, builder: &mut InstrSeqBuilder, structured: &ShapedBlock<u32>) {
        match structured {
            // a straight-line sequence of blocks
            // we just translate each one in turn
            ShapedBlock::Simple(simple) => {
                let can_branch_to = &simple.branches;

                build_wasm_block(
                    Block::from_u32(simple.label),
                    self.cursor,
                    builder,
                    &mut BlockCfCtx::new(
                        can_branch_to,
                        self.block_to_seq,
                        self.loop_to_block,
                        self.multi_to_block,
                    ),
                );

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
