mod linker;

use core::{
    convert::{AsMut, AsRef},
    ops::{Deref, DerefMut},
};
use intrusive_collections::RBTree;

pub use self::linker::{Linker, LinkerError};

use super::*;

/// A [Program] is a collection of [Module]s that are being compiled together as a package.
///
/// This is primarily used for storing/querying data which must be shared across modules:
///
/// * The set of global variables which will be allocated on the global heap
/// * The set of modules and functions which have been defined
///
/// When translating to Miden Assembly, we need something like this to allow us to perform some
/// basic linker tasks prior to emitting the textual MASM which will be fed to the Miden VM.
///
/// This structure is intended to be allocated via [std::sync::Arc], so that it can be shared
/// across multiple threads which are emitting/compiling modules at the same time. It is designed
/// so that individual fields are locked, rather than the structure as a whole, to minimize contention.
/// The intuition is that, in general, changes at the [Program] level are relatively infrequent, i.e.
/// only when declaring a new [Module], or [GlobalVariable], do we actually need to mutate the structure.
/// In all other situations, changes are scoped at the [Module] level.
#[derive(Default)]
pub struct Program {
    /// This tree stores all of the modules being compiled as part of the current program.
    modules: RBTree<ModuleTreeAdapter>,
    /// If set, this field indicates which function is the entrypoint for the program.
    ///
    /// When generating Miden Assembly, this will determine whether or not we're emitting
    /// a program or just a collection of modules; and in the case of the former, what code
    /// to emit in the root code block.
    entrypoint: Option<FunctionIdent>,
    /// The data segments gathered from all modules in the program, and laid out in address order.
    segments: DataSegmentTable,
    /// The global variable table produced by linking the global variable tables of all
    /// modules in this program. The layout of this table corresponds to the layout of
    /// global variables in the linear memory heap at runtime.
    globals: GlobalVariableTable,
}
impl Program {
    /// Create a new, empty [Program].
    #[inline(always)]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns true if this program has a defined entrypoint
    pub const fn has_entrypoint(&self) -> bool {
        self.entrypoint.is_none()
    }

    /// Returns true if this program is executable.
    ///
    /// An executable program is one which has an entrypoint that will be called
    /// after the program is loaded.
    pub const fn is_executable(&self) -> bool {
        self.has_entrypoint()
    }

    /// Returns the [FunctionIdent] corresponding to the program entrypoint
    pub fn entrypoint(&self) -> Option<FunctionIdent> {
        self.entrypoint
    }

    /// Return a reference to the module table for this program
    pub fn modules(&self) -> &RBTree<ModuleTreeAdapter> {
        &self.modules
    }

    /// Return a mutable reference to the module table for this program
    pub fn modules_mut(&mut self) -> &mut RBTree<ModuleTreeAdapter> {
        &mut self.modules
    }

    /// Return a reference to the data segment table for this program
    pub fn segments(&self) -> &DataSegmentTable {
        &self.segments
    }

    /// Get a reference to the global variable table for this program
    pub fn globals(&self) -> &GlobalVariableTable {
        &self.globals
    }

    /// Get a mutable reference to the global variable table for this program
    pub fn globals_mut(&mut self) -> &mut GlobalVariableTable {
        &mut self.globals
    }

    /// Returns true if `name` is defined in this program.
    pub fn contains(&self, name: Ident) -> bool {
        !self.modules.find(&name).is_null()
    }

    /// Look up the signature of a function in this program by `id`
    pub fn signature(&self, id: &FunctionIdent) -> Option<&Signature> {
        let module = self.modules.find(&id.module).get()?;
        module.function(id.function).map(|f| &f.signature)
    }
}

/// This struct provides an ergonomic way to construct a [Program] in an imperative fashion.
///
/// Simply create the builder, add/build one or more modules, then call `link` to obtain a [Program].
pub struct ProgramBuilder<'a> {
    modules: std::collections::BTreeMap<Ident, Box<Module>>,
    entry: Option<FunctionIdent>,
    diagnostics: &'a miden_diagnostics::DiagnosticsHandler,
}
impl<'a> ProgramBuilder<'a> {
    pub fn new(diagnostics: &'a miden_diagnostics::DiagnosticsHandler) -> Self {
        Self {
            modules: Default::default(),
            entry: None,
            diagnostics,
        }
    }

    /// Set the entrypoint for the [Program] being built.
    #[inline]
    pub fn with_entrypoint(mut self, id: FunctionIdent) -> Self {
        self.entry = Some(id);
        self
    }

    /// Add `module` to the set of modules to link into the final [Program]
    ///
    /// Unlike `add_module`, this function consumes the current builder state
    /// and returns a new one, to allow for chaining builder calls together.
    ///
    /// Returns `Err` if a module with the same name already exists
    pub fn with_module(mut self, module: Box<Module>) -> Result<Self, ModuleConflictError> {
        self.add_module(module).map(|_| self)
    }

    /// Add `module` to the set of modules to link into the final [Program]
    ///
    /// Returns `Err` if a module with the same name already exists
    pub fn add_module(&mut self, module: Box<Module>) -> Result<(), ModuleConflictError> {
        let module_name = module.name;
        if self.modules.contains_key(&module_name) {
            return Err(ModuleConflictError(module_name));
        }

        self.modules.insert(module_name, module);

        Ok(())
    }

    /// Start building a [Module] with the given name.
    ///
    /// When the builder is done, the resulting [Module] will be inserted
    /// into the set of modules to be linked into the final [Program].
    pub fn module<S: Into<Ident>>(&mut self, name: S) -> ProgramModuleBuilder<'_, 'a> {
        let name = name.into();
        let module = match self.modules.remove(&name) {
            None => Box::new(Module::new(name)),
            Some(module) => module,
        };
        ProgramModuleBuilder {
            pb: self,
            mb: ModuleBuilder::from(module),
        }
    }

    /// Link a [Program] from the current [ProgramBuilder] state
    pub fn link(self) -> Result<Box<Program>, LinkerError> {
        let mut linker = Linker::new();
        if let Some(entry) = self.entry {
            linker.with_entrypoint(entry)?;
        }

        for (_, module) in self.modules.into_iter() {
            linker.add(module)?;
        }

        linker.link()
    }
}

/// This is used to build a [Module] from a [ProgramBuilder].
///
/// It is basically just a wrapper around [ModuleBuilder], but overrides two things:
///
/// * `build` will add the module to the [ProgramBuilder] directly, rather than returning it
/// * `function` will delegate to [ProgramFunctionBuilder] which plays a similar role to this
/// struct, but for [ModuleFunctionBuilder].
pub struct ProgramModuleBuilder<'a, 'b: 'a> {
    pb: &'a mut ProgramBuilder<'b>,
    mb: ModuleBuilder,
}
impl<'a, 'b: 'a> ProgramModuleBuilder<'a, 'b> {
    /// Start building a [Function] wwith the given name and signature.
    pub fn function<'c, 'd: 'c, S: Into<Ident>>(
        &'d mut self,
        name: S,
        signature: Signature,
    ) -> Result<ProgramFunctionBuilder<'c, 'd>, SymbolConflictError> {
        Ok(ProgramFunctionBuilder {
            diagnostics: self.pb.diagnostics,
            fb: self.mb.function(name, signature)?,
        })
    }

    /// Build the current [Module], adding it to the [ProgramBuilder].
    ///
    /// Returns `err` if a module with that name already exists.
    pub fn build(self) -> Result<(), ModuleConflictError> {
        let pb = self.pb;
        let mb = self.mb;

        pb.add_module(mb.build())?;
        Ok(())
    }
}
impl<'a, 'b: 'a> Deref for ProgramModuleBuilder<'a, 'b> {
    type Target = ModuleBuilder;

    fn deref(&self) -> &Self::Target {
        &self.mb
    }
}
impl<'a, 'b: 'a> DerefMut for ProgramModuleBuilder<'a, 'b> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.mb
    }
}
impl<'a, 'b: 'a> AsRef<ModuleBuilder> for ProgramModuleBuilder<'a, 'b> {
    fn as_ref(&self) -> &ModuleBuilder {
        &self.mb
    }
}
impl<'a, 'b: 'a> AsMut<ModuleBuilder> for ProgramModuleBuilder<'a, 'b> {
    fn as_mut(&mut self) -> &mut ModuleBuilder {
        &mut self.mb
    }
}

/// This is used to build a [Function] from a [ProgramModuleBuilder].
///
/// It is basically just a wrapper around [ModuleFunctionBuilder], but overrides
/// `build` to use the [miden_diagnostics::DiagnosticsHandler] of the parent
/// [ProgramBuilder].
pub struct ProgramFunctionBuilder<'a, 'b: 'a> {
    diagnostics: &'b miden_diagnostics::DiagnosticsHandler,
    fb: ModuleFunctionBuilder<'a>,
}
impl<'a, 'b: 'a> ProgramFunctionBuilder<'a, 'b> {
    /// Build the current function
    pub fn build(self) -> Result<FunctionIdent, InvalidFunctionError> {
        let diagnostics = self.diagnostics;
        self.fb.build(diagnostics)
    }
}
impl<'a, 'b: 'a> Deref for ProgramFunctionBuilder<'a, 'b> {
    type Target = ModuleFunctionBuilder<'a>;

    fn deref(&self) -> &Self::Target {
        &self.fb
    }
}
impl<'a, 'b: 'a> DerefMut for ProgramFunctionBuilder<'a, 'b> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.fb
    }
}
impl<'a, 'b: 'a> AsRef<ModuleFunctionBuilder<'a>> for ProgramFunctionBuilder<'a, 'b> {
    fn as_ref(&self) -> &ModuleFunctionBuilder<'a> {
        &self.fb
    }
}
impl<'a, 'b: 'a> AsMut<ModuleFunctionBuilder<'a>> for ProgramFunctionBuilder<'a, 'b> {
    fn as_mut(&mut self) -> &mut ModuleFunctionBuilder<'a> {
        &mut self.fb
    }
}
