use std::fmt;

use cranelift_entity::{EntityRef, PrimaryMap};
use intrusive_collections::{intrusive_adapter, LinkedListLink};
use miden_diagnostics::Spanned;
use miden_hir::{FunctionIdent, Signature, Type};
use rustc_hash::FxHashMap;
use smallvec::{smallvec, SmallVec};

use super::*;

intrusive_adapter!(pub FunctionListAdapter = Box<Function>: Function { link: LinkedListLink });

/// This represents a function in Miden Assembly
#[derive(Spanned)]
pub struct Function {
    link: LinkedListLink,
    /// The name of this function
    #[span]
    pub name: FunctionIdent,
    /// The type signature of this function
    pub signature: Signature,
    /// The root block of code for this function
    pub body: BlockId,
    /// Storage for the blocks of code in this function's body
    pub blocks: PrimaryMap<BlockId, Block>,
    /// Locals allocated for this function
    locals: SmallVec<[Local; 1]>,
    /// The next available local index
    next_local_id: usize,
}
impl Function {
    pub fn new(name: FunctionIdent, signature: Signature) -> Self {
        let mut blocks = PrimaryMap::<BlockId, Block>::default();
        let body_id = blocks.next_key();
        let body = blocks.push(Block {
            id: body_id,
            ops: smallvec![],
        });
        Self {
            link: Default::default(),
            name,
            signature,
            body,
            blocks,
            locals: Default::default(),
            next_local_id: 0,
        }
    }

    /// Return the number of arguments expected on the operand stack
    #[inline]
    pub fn arity(&self) -> usize {
        self.signature.arity()
    }

    /// Return the number of results produced by this function
    #[inline]
    pub fn num_results(&self) -> usize {
        self.signature.results.len()
    }

    /// Allocate a new local in this function, using the provided data
    ///
    /// The index of the local is returned as it's identifier
    pub fn alloc_local(&mut self, ty: Type) -> LocalId {
        let num_words = ty.size_in_words();
        let next_id = self.next_local_id;
        assert!(
            (next_id + num_words) < (u8::MAX as usize),
            "unable to allocate a local of type {}: unable to allocate enough local memory",
            &ty
        );
        let id = LocalId::new(next_id);
        self.next_local_id += num_words;
        let local = Local { id, ty };
        self.locals.push(local);
        id
    }

    /// Get the local with the given identifier
    pub fn local(&self, id: LocalId) -> &Local {
        self.locals
            .iter()
            .find(|l| l.id == id)
            .expect("invalid local id")
    }

    /// Return the locals allocated in this function as a slice
    #[inline]
    pub fn locals(&self) -> &[Local] {
        self.locals.as_slice()
    }

    /// Allocate a new code block in this function
    pub fn create_block(&mut self) -> BlockId {
        let id = self.blocks.next_key();
        self.blocks.push(Block {
            id,
            ops: smallvec![],
        });
        id
    }

    #[inline]
    pub fn block(&self, id: BlockId) -> &Block {
        &self.blocks[id]
    }

    #[inline]
    pub fn block_mut(&mut self, id: BlockId) -> &mut Block {
        &mut self.blocks[id]
    }

    /// Return an implementation of [std::fmt::Display] for this function
    pub fn display<'a, 'b: 'a>(&'b self, imports: &'b ModuleImportInfo) -> DisplayMasmFunction<'a> {
        DisplayMasmFunction {
            function: self,
            imports,
        }
    }

    pub fn to_function_ast(
        &self,
        codemap: &miden_diagnostics::CodeMap,
        imports: &miden_hir::ModuleImportInfo,
        local_ids: &FxHashMap<FunctionIdent, u16>,
        proc_ids: &FxHashMap<FunctionIdent, miden_assembly::ProcedureId>,
    ) -> miden_assembly::ast::ProcedureAst {
        use miden_assembly::{
            self as masm,
            ast::{ProcedureAst, SourceLocation},
        };

        let name = masm::ProcedureName::try_from(self.name.function.as_str())
            .expect("invalid function name");
        let num_locals = u16::try_from(self.locals.len()).expect("too many locals");
        let start = codemap
            .location(self)
            .ok()
            .map(|loc| {
                SourceLocation::new(loc.line.to_usize() as u32, loc.column.to_usize() as u32)
            })
            .unwrap_or_default();
        let body = emit_block(
            self.body,
            &self.blocks,
            codemap,
            imports,
            local_ids,
            proc_ids,
        );

        ProcedureAst {
            name,
            docs: None,
            num_locals,
            body,
            start,
            is_export: self.signature.is_public(),
        }
    }
}

fn emit_block(
    block_id: BlockId,
    blocks: &PrimaryMap<BlockId, Block>,
    codemap: &miden_diagnostics::CodeMap,
    imports: &miden_hir::ModuleImportInfo,
    local_ids: &FxHashMap<FunctionIdent, u16>,
    proc_ids: &FxHashMap<FunctionIdent, miden_assembly::ProcedureId>,
) -> miden_assembly::ast::CodeBody {
    use miden_assembly::ast::{CodeBody, Node};

    let current_block = &blocks[block_id];
    let mut ops = Vec::with_capacity(current_block.ops.len());
    for op in current_block.ops.iter() {
        match op.clone() {
            Op::If(then_blk, else_blk) => {
                let true_case = emit_block(then_blk, blocks, codemap, imports, local_ids, proc_ids);
                let false_case =
                    emit_block(else_blk, blocks, codemap, imports, local_ids, proc_ids);
                ops.push(Node::IfElse {
                    true_case,
                    false_case,
                });
            }
            Op::While(blk) => {
                let body = emit_block(blk, blocks, codemap, imports, local_ids, proc_ids);
                ops.push(Node::While { body });
            }
            Op::Repeat(n, blk) => {
                let body = emit_block(blk, blocks, codemap, imports, local_ids, proc_ids);
                ops.push(Node::Repeat {
                    times: n as u32,
                    body,
                });
            }
            op => {
                ops.extend(op.into_node(codemap, imports, local_ids, proc_ids));
            }
        }
    }

    CodeBody::new(ops)
}

impl fmt::Debug for Function {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("Function")
            .field("name", &self.name)
            .field("body", &self.body)
            .field("blocks", &self.blocks)
            .finish()
    }
}

#[doc(hidden)]
pub struct DisplayMasmFunction<'a> {
    function: &'a Function,
    imports: &'a ModuleImportInfo,
}
impl<'a> fmt::Display for DisplayMasmFunction<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use miden_hir::DisplayMasmBlock;

        let visibility = if self.function.signature.is_public() {
            "export"
        } else {
            "proc"
        };
        let name = self.function.name;
        match self.function.locals.len() {
            0 => {
                writeln!(f, "{visibility}.{}", &name.function)?;
            }
            n => {
                writeln!(f, "{visibility}.{}.{}", &name.function, n)?;
            }
        }

        writeln!(
            f,
            "{}",
            DisplayMasmBlock::new(
                Some(self.imports),
                &self.function.blocks,
                self.function.body,
                1
            )
        )?;

        f.write_str("end")
    }
}
