//! A WebAssembly module for Cranelift.

#[cfg(test)]
mod tests;

mod conversions;
mod optable;

use std::path::Path;

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
use optable::OperandTable;
use relooper::{reloop, ShapedBlock};
use wabt::wasm2wat;
use walrus::{
    ir::{BinaryOp, InstrSeqId},
    DataKind, FunctionBuilder, InstrSeqBuilder, LocalId, MemoryId, Module as WalrusModule,
    ModuleConfig, ModuleLocals, ValType,
};

use crate::conversions::{
    block::{build_wasm_block, BranchInstr, CanBranchTo},
    sig::wasm_of_sig,
    ty,
};

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
    /// Maps Cranelift functions to Walrus functions.
    functions: FnvHashMap<FuncId, walrus::FunctionId>,
    /// Maps Cranelift data items to Walrus data items.
    data: FnvHashMap<DataId, walrus::DataId>,
}

impl WasmModule {
    /// Constructs a new WebAssembly module.
    ///
    /// todo: check the target isa (or maybe don't take it as a parameter, and generate it instead)
    pub fn new(config: ModuleConfig) -> Self {
        // if !matches!(
        //     isa.triple().binary_format,
        //     target_lexicon::BinaryFormat::Wasm
        // ) {
        //     panic!(
        //         "only WebAssembly is supported! for other targets, you may want to look at
        //     `cranelift_object`."
        //     )
        // }

        let mut module = WalrusModule::default();

        let memory_id = module.memories.add_local(false, 1000, None);

        Self {
            decls: Default::default(),
            module,
            config,
            memory_id,
            functions: Default::default(),
            data: Default::default(),
        }
    }

    /// Emit the module as it stands as a series of bytes (which can be interpreted as a
    /// WebAssembly module).
    pub fn emit(&mut self) -> Vec<u8> {
        self.module.emit_wasm()
    }

    pub fn emit_wat(&mut self) -> String {
        let wasm = self.emit();
        wasm2wat(&wasm).unwrap()
    }

    /// Renders the current module as a graphviz dot file.
    pub fn render_graphviz(&self, path: impl AsRef<Path>) {
        self.module
            .write_graphviz_dot(path)
            .expect("failed to write graphviz file");
    }
}

impl CraneliftModule for WasmModule {
    fn isa(&self) -> &dyn TargetIsa {
        panic!("the WebAssembly ISA still needs to be defined");
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
        let (clif_id, _) = self.decls.declare_function(name, linkage, signature)?;

        let (params, ret) = wasm_of_sig(signature.clone());

        match linkage {
            Linkage::Import => todo!(),
            Linkage::Local => {
                let mut builder = FunctionBuilder::new(&mut self.module.types, &params, &ret);
                builder.name(name.to_string());
                // todo: handle args
                let local = builder.finish(vec![], &mut self.module.funcs);
                self.functions.insert(clif_id, local);
            }
            Linkage::Preemptible | Linkage::Hidden => unimplemented!(),
            Linkage::Export => {
                let mut builder = FunctionBuilder::new(&mut self.module.types, &params, &ret);
                builder.name(name.to_string());
                // todo: handle args
                let local = builder.finish(vec![], &mut self.module.funcs);
                self.functions.insert(clif_id, local);
                self.module.exports.add(name, local);
            }
        }

        Ok(clif_id)
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
        let clif_data_id = self.decls.declare_anonymous_data(writable, tls)?;
        let walrus_data_id = self.module.data.add(DataKind::Passive, Vec::new());
        self.data.insert(clif_data_id, walrus_data_id);
        Ok(clif_data_id)
    }

    fn define_function(
        &mut self,
        func_id: FuncId,
        ctx: &mut Context,
        _trap_sink: &mut dyn binemit::TrapSink,
        _stack_map_sink: &mut dyn binemit::StackMapSink,
    ) -> ModuleResult<ModuleCompiledFunction> {
        log::trace!("started compiling function with id {:#?}", func_id);

        let id = self
            .functions
            .get(&func_id)
            .expect("function declared but never defined!");

        log::trace!("module details: {:#?}", self.module);

        // retrieve WebAssembly function
        let func = self.module.funcs.get_mut(*id);

        log::trace!("found function: {:#?}", func);
        let mut builder = match func.kind {
            walrus::FunctionKind::Import(_) => unreachable!(),
            walrus::FunctionKind::Local(ref mut loc) => loc.builder_mut().func_body(),
            walrus::FunctionKind::Uninitialized(_) => unreachable!(),
        };

        // set up Cranelift
        let mut cursor = FuncCursor::new(&mut ctx.func);

        let operand_table = OperandTable::fill(&mut cursor, &mut self.module.locals);

        log::trace!("computed operand table: {:#?}", operand_table);

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
                    BranchInfo::Table(_, _) => {
                        todo!()
                    }
                }
            }

            relooper_blocks.push((block.as_u32(), branches))
        }

        log::trace!("generated relooper input: {:#?}", relooper_blocks);

        let first = cursor.func.layout.entry_block().unwrap().as_u32();

        let structured = reloop(relooper_blocks, first);

        log::trace!("recovered control flow: {:#?}", structured);

        let (mut block_to_seq, mut loop_to_block, mut multi_to_block) =
            (Default::default(), Default::default(), Default::default());

        let mut locals: FnvHashMap<_, _> = Default::default();

        let params = cursor
            .func
            .dfg
            .block_params(cursor.layout().entry_block().unwrap());
        for each in params {
            let ty = cursor.func.dfg.value_type(*each);
            let local_id = self.module.locals.add(ty::wasm_of_cranelift(ty));
            locals.insert(*each, local_id);
        }

        let mut translator = IndividualFunctionTranslator::new(
            &mut self.module.locals,
            &mut cursor,
            &mut block_to_seq,
            &mut loop_to_block,
            &mut multi_to_block,
            &operand_table,
            &mut locals,
        );

        translator.compile_structured(&mut builder, &structured, None, has_next(&structured));
        builder.unreachable();

        log::trace!("finished compiling func with id {:#?}", func_id);

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
    module_locals: &'clif mut ModuleLocals,
    /// The cursor which we are using to query useful relevant information from Cranelift about the
    /// nature of the IR with which we are being provided.
    cursor: &'clif mut FuncCursor<'clif>,
    // todo: is this one even needed?
    #[allow(unused)]
    block_to_seq: &'clif mut FnvHashMap<Block, InstrSeqId>,
    /// stores the `InstrSeqId`s that loops correspond to.
    loop_to_block: &'clif mut FnvHashMap<u16, InstrSeqId>,
    #[allow(unused)]
    multi_to_block: &'clif mut FnvHashMap<u16, InstrSeqId>,
    operand_table: &'clif OperandTable,
    locals: &'clif mut FnvHashMap<ir::Value, LocalId>,
}

impl<'clif> IndividualFunctionTranslator<'clif> {
    fn new(
        module: &'clif mut ModuleLocals,
        cursor: &'clif mut FuncCursor<'clif>,
        block_to_seq: &'clif mut FnvHashMap<Block, InstrSeqId>,
        loop_to_block: &'clif mut FnvHashMap<u16, InstrSeqId>,
        multi_to_block: &'clif mut FnvHashMap<u16, InstrSeqId>,
        operand_table: &'clif OperandTable,
        locals: &'clif mut FnvHashMap<ir::Value, LocalId>,
    ) -> Self {
        Self {
            module_locals: module,
            cursor,
            block_to_seq,
            loop_to_block,
            multi_to_block,
            operand_table,
            locals,
        }
    }

    fn compile_structured(
        &mut self,
        builder: &mut InstrSeqBuilder,
        structured: &ShapedBlock<u32>,
        label: Option<LocalId>,
        next_exists: bool,
    ) {
        log::trace!("compiling structured: {:#?}", structured);
        match structured {
            // a straight-line sequence of blocks
            // we just translate each one in turn
            ShapedBlock::Simple(simple) => {
                log::trace!("structured was a simple block: {:#?}", simple);
                if let Some(ref immediate) = simple.immediate {
                    let local = self.module_locals.add(ValType::I32);

                    let mut locally_computed = FnvHashMap::default();
                    if let ShapedBlock::Multiple(block) = immediate.as_ref() {
                        for each in &block.handled {
                            for label in &each.labels {
                                locally_computed.insert(*label, BranchInstr::SetLocal(local));
                            }
                        }
                    }

                    log::trace!(
                        "computed blocks that can be branched to with a label: {:#?}",
                        locally_computed
                    );

                    build_wasm_block(
                        Block::from_u32(simple.label),
                        self,
                        builder,
                        &CanBranchTo {
                            from_relooper: &simple.branches,
                            locally_computed,
                        },
                    );

                    self.compile_structured(builder, immediate, Some(local), has_next(&structured));
                } else {
                    build_wasm_block(
                        Block::from_u32(simple.label),
                        self,
                        builder,
                        &CanBranchTo {
                            from_relooper: &simple.branches,
                            locally_computed: Default::default(),
                        },
                    );
                }

                if let Some(ref next) = simple.next {
                    self.compile_structured(builder, next, None, has_next(next));
                }
            }
            ShapedBlock::Loop(l) => {
                log::trace!("structured was a loop: {:#?}", l);

                builder.loop_(None, |builder: &mut InstrSeqBuilder| {
                    self.loop_to_block.insert(l.loop_id, builder.id());
                    log::trace!("added `{:?}={:?}`", l.loop_id, builder.id());
                    self.compile_structured(builder, &l.inner, None, has_next(&l.inner));
                });

                if let Some(ref next) = l.next {
                    self.compile_structured(builder, next, None, has_next(&next));
                }
            }
            // `match`/`if` + `else if` chain
            ShapedBlock::Multiple(m) => {
                log::trace!("structured was a multiple block: {:#?}", m);

                // note: `HandledBlock::break_after` means "can this entry reach another entry"

                let label = label.unwrap();

                // now we run the if-else sequence
                // once for each node in the state machine
                for each in &m.handled {
                    // and then once for each possible label
                    for val in &each.labels {
                        builder
                            // check if the `label` local matches the id in question
                            .local_get(label)
                            .i32_const(*val as i32)
                            .binop(BinaryOp::I32Eq)
                            .if_else(
                                None,
                                |builder| {
                                    self.compile_structured(
                                        builder,
                                        &each.inner,
                                        None,
                                        has_next(&each.inner),
                                    );
                                },
                                |_| {},
                            );
                    }
                }

                if !next_exists {
                    builder.unreachable();
                }
            }
        }
    }
}

fn has_next(shaped: &ShapedBlock<u32>) -> bool {
    match shaped {
        ShapedBlock::Simple(s) => s.next.is_some(),
        ShapedBlock::Loop(l) => l.next.is_some(),
        ShapedBlock::Multiple(_) => false,
    }
}
