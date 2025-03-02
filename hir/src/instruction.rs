use std::{
    convert::{AsMut, AsRef},
    fmt,
    ops::{Deref, DerefMut},
};

use cranelift_entity::entity_impl;
use intrusive_collections::{intrusive_adapter, LinkedListLink, UnsafeRef};
use smallvec::SmallVec;

use miden_diagnostics::{Span, Spanned};

use super::*;

/// A handle to a single instruction
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Inst(u32);
entity_impl!(Inst, "inst");

/// Represents the data associated with an `Inst`.
///
/// Specifically, this represents a leaf node in the control flow graph of
/// a function, i.e. it links a specific instruction in to the sequence of
/// instructions belonging to a specific block.
#[derive(Spanned)]
pub struct InstNode {
    pub link: LinkedListLink,
    pub key: Inst,
    pub block: Block,
    #[span]
    pub data: Span<Instruction>,
}
impl fmt::Debug for InstNode {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", &self.data)
    }
}
impl InstNode {
    pub fn new(key: Inst, block: Block, data: Span<Instruction>) -> Self {
        Self {
            link: LinkedListLink::default(),
            key,
            block,
            data,
        }
    }

    pub fn deep_clone(&self, value_lists: &mut ValueListPool) -> Self {
        let span = self.data.span();
        Self {
            link: LinkedListLink::default(),
            key: self.key,
            block: self.block,
            data: Span::new(span, self.data.deep_clone(value_lists)),
        }
    }

    pub fn replace(&mut self, data: Span<Instruction>) {
        self.data = data;
    }
}
impl Deref for InstNode {
    type Target = Instruction;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.data
    }
}
impl DerefMut for InstNode {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.data
    }
}
impl AsRef<Instruction> for InstNode {
    #[inline]
    fn as_ref(&self) -> &Instruction {
        &self.data
    }
}
impl AsMut<Instruction> for InstNode {
    #[inline]
    fn as_mut(&mut self) -> &mut Instruction {
        &mut self.data
    }
}

intrusive_adapter!(pub InstAdapter = UnsafeRef<InstNode>: InstNode { link: LinkedListLink });

/// Represents the type of instruction associated with a particular opcode
#[derive(Debug)]
pub enum Instruction {
    GlobalValue(GlobalValueOp),
    BinaryOp(BinaryOp),
    BinaryOpImm(BinaryOpImm),
    UnaryOp(UnaryOp),
    UnaryOpImm(UnaryOpImm),
    Call(Call),
    Br(Br),
    CondBr(CondBr),
    Switch(Switch),
    Ret(Ret),
    RetImm(RetImm),
    Load(LoadOp),
    PrimOp(PrimOp),
    PrimOpImm(PrimOpImm),
    Test(Test),
    InlineAsm(InlineAsm),
}
impl Instruction {
    pub fn deep_clone(&self, value_lists: &mut ValueListPool) -> Self {
        match self {
            Self::GlobalValue(gv) => Self::GlobalValue(gv.clone()),
            Self::BinaryOp(op) => Self::BinaryOp(op.clone()),
            Self::BinaryOpImm(op) => Self::BinaryOpImm(op.clone()),
            Self::UnaryOp(op) => Self::UnaryOp(op.clone()),
            Self::UnaryOpImm(op) => Self::UnaryOpImm(op.clone()),
            Self::Call(call) => Self::Call(Call {
                args: call.args.deep_clone(value_lists),
                ..call.clone()
            }),
            Self::Br(br) => Self::Br(Br {
                args: br.args.deep_clone(value_lists),
                ..br.clone()
            }),
            Self::CondBr(br) => Self::CondBr(CondBr {
                then_dest: (br.then_dest.0, br.then_dest.1.deep_clone(value_lists)),
                else_dest: (br.else_dest.0, br.else_dest.1.deep_clone(value_lists)),
                ..br.clone()
            }),
            Self::Switch(op) => Self::Switch(op.clone()),
            Self::Ret(op) => Self::Ret(Ret {
                args: op.args.deep_clone(value_lists),
                ..op.clone()
            }),
            Self::RetImm(op) => Self::RetImm(op.clone()),
            Self::Load(op) => Self::Load(op.clone()),
            Self::PrimOp(op) => Self::PrimOp(PrimOp {
                args: op.args.deep_clone(value_lists),
                ..op.clone()
            }),
            Self::PrimOpImm(op) => Self::PrimOpImm(PrimOpImm {
                args: op.args.deep_clone(value_lists),
                ..op.clone()
            }),
            Self::Test(op) => Self::Test(op.clone()),
            Self::InlineAsm(op) => Self::InlineAsm(InlineAsm {
                args: op.args.deep_clone(value_lists),
                ..op.clone()
            }),
        }
    }

    pub fn opcode(&self) -> Opcode {
        match self {
            Self::GlobalValue(GlobalValueOp { ref op, .. })
            | Self::BinaryOp(BinaryOp { ref op, .. })
            | Self::BinaryOpImm(BinaryOpImm { ref op, .. })
            | Self::UnaryOp(UnaryOp { ref op, .. })
            | Self::UnaryOpImm(UnaryOpImm { ref op, .. })
            | Self::Call(Call { ref op, .. })
            | Self::Br(Br { ref op, .. })
            | Self::CondBr(CondBr { ref op, .. })
            | Self::Switch(Switch { ref op, .. })
            | Self::Ret(Ret { ref op, .. })
            | Self::RetImm(RetImm { ref op, .. })
            | Self::Load(LoadOp { ref op, .. })
            | Self::PrimOp(PrimOp { ref op, .. })
            | Self::PrimOpImm(PrimOpImm { ref op, .. })
            | Self::Test(Test { ref op, .. })
            | Self::InlineAsm(InlineAsm { ref op, .. }) => *op,
        }
    }

    /// Returns true if this instruction has side effects, or may have side effects
    ///
    /// Side effects are defined as control flow, writing memory, trapping execution,
    /// I/O, etc.
    ///
    #[inline]
    pub fn has_side_effects(&self) -> bool {
        self.opcode().has_side_effects()
    }

    /// Returns true if this instruction is a binary operator requiring two operands
    ///
    /// NOTE: Binary operators with immediate operands are not considered binary for
    /// this purpose, as they only require a single operand to be provided to the
    /// instruction, the immediate being the other one provided by the instruction
    /// itself.
    pub fn is_binary(&self) -> bool {
        matches!(self, Self::BinaryOp(_))
    }

    /// Returns true if this instruction is a binary operator whose operands may
    /// appear in any order.
    #[inline]
    pub fn is_commutative(&self) -> bool {
        self.opcode().is_commutative()
    }

    pub fn arguments<'a>(&'a self, pool: &'a ValueListPool) -> &[Value] {
        match self {
            Self::BinaryOp(BinaryOp { ref args, .. }) => args.as_slice(),
            Self::BinaryOpImm(BinaryOpImm { ref arg, .. }) => core::slice::from_ref(arg),
            Self::UnaryOp(UnaryOp { ref arg, .. }) => core::slice::from_ref(arg),
            Self::Call(Call { ref args, .. }) => args.as_slice(pool),
            Self::CondBr(CondBr { ref cond, .. }) => core::slice::from_ref(cond),
            Self::Switch(Switch { ref arg, .. }) => core::slice::from_ref(arg),
            Self::Ret(Ret { ref args, .. }) => args.as_slice(pool),
            Self::Load(LoadOp { ref addr, .. }) => core::slice::from_ref(addr),
            Self::PrimOp(PrimOp { ref args, .. }) => args.as_slice(pool),
            Self::PrimOpImm(PrimOpImm { ref args, .. }) => args.as_slice(pool),
            Self::Test(Test { ref arg, .. }) => core::slice::from_ref(arg),
            Self::InlineAsm(InlineAsm { ref args, .. }) => args.as_slice(pool),
            Self::GlobalValue(_) | Self::UnaryOpImm(_) | Self::Br(_) | Self::RetImm(_) => &[],
        }
    }

    pub fn arguments_mut<'a>(&'a mut self, pool: &'a mut ValueListPool) -> &mut [Value] {
        match self {
            Self::BinaryOp(BinaryOp { ref mut args, .. }) => args.as_mut_slice(),
            Self::BinaryOpImm(BinaryOpImm { ref mut arg, .. }) => core::slice::from_mut(arg),
            Self::UnaryOp(UnaryOp { ref mut arg, .. }) => core::slice::from_mut(arg),
            Self::Call(Call { ref mut args, .. }) => args.as_mut_slice(pool),
            Self::CondBr(CondBr { ref mut cond, .. }) => core::slice::from_mut(cond),
            Self::Switch(Switch { ref mut arg, .. }) => core::slice::from_mut(arg),
            Self::Ret(Ret { ref mut args, .. }) => args.as_mut_slice(pool),
            Self::Load(LoadOp { ref mut addr, .. }) => core::slice::from_mut(addr),
            Self::PrimOp(PrimOp { ref mut args, .. }) => args.as_mut_slice(pool),
            Self::PrimOpImm(PrimOpImm { ref mut args, .. }) => args.as_mut_slice(pool),
            Self::Test(Test { ref mut arg, .. }) => core::slice::from_mut(arg),
            Self::InlineAsm(InlineAsm { ref mut args, .. }) => args.as_mut_slice(pool),
            Self::GlobalValue(_) | Self::UnaryOpImm(_) | Self::Br(_) | Self::RetImm(_) => &mut [],
        }
    }

    pub fn analyze_branch<'a>(&'a self, pool: &'a ValueListPool) -> BranchInfo<'a> {
        match self {
            Self::Br(ref b) => BranchInfo::SingleDest(b.destination, b.args.as_slice(pool)),
            Self::CondBr(CondBr {
                ref then_dest,
                ref else_dest,
                ..
            }) => BranchInfo::MultiDest(vec![
                JumpTable::new(then_dest.0, then_dest.1.as_slice(pool)),
                JumpTable::new(else_dest.0, else_dest.1.as_slice(pool)),
            ]),
            Self::Switch(Switch {
                ref arms,
                ref default,
                ..
            }) => {
                let mut targets = arms
                    .iter()
                    .map(|(_, b)| JumpTable::new(*b, &[]))
                    .collect::<Vec<_>>();
                targets.push(JumpTable::new(*default, &[]));
                BranchInfo::MultiDest(targets)
            }
            _ => BranchInfo::NotABranch,
        }
    }

    pub fn analyze_call<'a>(&'a self, pool: &'a ValueListPool) -> CallInfo<'a> {
        match self {
            Self::Call(ref c) => CallInfo::Direct(c.callee, c.args.as_slice(pool)),
            _ => CallInfo::NotACall,
        }
    }
}

#[derive(Debug)]
pub enum BranchInfo<'a> {
    NotABranch,
    SingleDest(Block, &'a [Value]),
    MultiDest(Vec<JumpTable<'a>>),
}

#[derive(Debug)]
pub struct JumpTable<'a> {
    pub destination: Block,
    pub args: &'a [Value],
}
impl<'a> JumpTable<'a> {
    pub fn new(destination: Block, args: &'a [Value]) -> Self {
        Self { destination, args }
    }
}

pub enum CallInfo<'a> {
    NotACall,
    Direct(FunctionIdent, &'a [Value]),
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum Opcode {
    /// Asserts the given value is 1
    Assert,
    /// Asserts the given value is 0
    Assertz,
    /// Asserts the two given values are equal
    AssertEq,
    /// Represents an immediate boolean value (1-bit integer)
    ImmI1,
    /// Represents an immediate unsigned 8-bit integer value
    ImmU8,
    /// Represents an immediate signed 8-bit integer value
    ImmI8,
    /// Represents an immediate unsigned 16-bit integer value
    ImmU16,
    /// Represents an immediate signed 16-bit integer value
    ImmI16,
    /// Represents an immediate unsigned 32-bit integer value
    ImmU32,
    /// Represents an immediate signed 32-bit integer value
    ImmI32,
    /// Represents an immediate unsigned 64-bit integer value
    ImmU64,
    /// Represents an immediate signed 64-bit integer value
    ImmI64,
    /// Represents an immediate field element
    ImmFelt,
    /// Represents an immediate 64-bit floating-point value
    ImmF64,
    /// Allocates a new "null" value in a temporary memory slot, where null is defined by
    /// the semantics of the type. The result of this instruction is always a pointer to
    /// the allocated type.
    ///
    /// For integral types, the null value is always zero.
    ///
    /// For pointer types, the null value is equal to the address of the start of the linear
    /// memory range, i.e. address `0x0`.
    ///
    /// For structs and arrays, the null value is a value equal in size (in bytes) to the size
    /// of the type, but whose contents are undefined, i.e. you cannot assume that the binary
    /// representation of the value is zeroed.
    Alloca,
    /// Like the WebAssembly `memory.grow` instruction, this allocates a given number of pages from the
    /// global heap, and returns the previous size of the heap, in pages. Each page is 64kb by default.
    ///
    /// For the time being, this instruction is emulated using a heap pointer global which tracks
    /// the "end" of the available heap. Nothing actually prevents one from accessing memory past
    /// that point (assuming it is within the 32-bit address range), however this allows us to
    /// support code compiled for the `wasm32-unknown-unknown` target cleanly.
    MemGrow,
    /// This instruction is used to represent a global value in the IR
    ///
    /// See [GlobalValueOp] and [GlobalValueData] for details on what types of values are represented
    /// behind this opcode.
    GlobalValue,
    /// Loads a value from a pointer to memory
    Load,
    /// Stores a value to a pointer to memory
    Store,
    /// Copies `n` values of a given type from a source pointer to a destination pointer
    MemCpy,
    /// Casts a pointer value to an integral type
    PtrToInt,
    /// Casts an integral type to a pointer value
    IntToPtr,
    /// Casts from a field element type to an integral type
    ///
    /// It is not valid to perform a cast on any value other than a field element, see
    /// `Trunc`, `Zext`, and `Sext` for casts between machine integer types.
    Cast,
    /// Truncates a larger integral type to a smaller integral type, e.g. i64 -> i32
    Trunc,
    /// Zero-extends a smaller unsigned integral type to a larger unsigned integral type, e.g. u32 -> u64
    Zext,
    /// Sign-extends a smaller signed integral type to a larger signed integral type, e.g. i32 -> i64
    Sext,
    /// Returns true if argument fits in the given integral type, e.g. u32, otherwise false
    Test,
    /// Selects between two values given a conditional
    Select,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    DivMod,
    Neg,
    Inv,
    Incr,
    Pow2,
    Exp,
    Not,
    Bnot,
    And,
    Band,
    Or,
    Bor,
    Xor,
    Bxor,
    Shl,
    Shr,
    Rotl,
    Rotr,
    Popcnt,
    Eq,
    Neq,
    Gt,
    Gte,
    Lt,
    Lte,
    IsOdd,
    Min,
    Max,
    Call,
    Syscall,
    Br,
    CondBr,
    Switch,
    Ret,
    Unreachable,
    InlineAsm,
}
impl Opcode {
    pub fn is_terminator(&self) -> bool {
        matches!(
            self,
            Self::Br | Self::CondBr | Self::Switch | Self::Ret | Self::Unreachable
        )
    }

    pub fn is_branch(&self) -> bool {
        matches!(self, Self::Br | Self::CondBr | Self::Switch)
    }

    pub fn is_call(&self) -> bool {
        matches!(self, Self::Call | Self::Syscall)
    }

    pub fn is_commutative(&self) -> bool {
        matches!(
            self,
            Self::Add
                | Self::Mul
                | Self::Min
                | Self::Max
                | Self::Eq
                | Self::Neq
                | Self::And
                | Self::Band
                | Self::Or
                | Self::Bor
                | Self::Xor
                | Self::Bxor
        )
    }

    pub fn has_side_effects(&self) -> bool {
        match self {
            // These opcodes are all effectful
            Self::Assert
            | Self::Assertz
            | Self::AssertEq
            | Self::Store
            | Self::Alloca
            | Self::MemCpy
            | Self::MemGrow
            | Self::Call
            | Self::Syscall
            | Self::Br
            | Self::CondBr
            | Self::Switch
            | Self::Ret
            | Self::Unreachable
            | Self::InlineAsm => true,
            // These opcodes are not
            Self::ImmI1
            | Self::ImmU8
            | Self::ImmI8
            | Self::ImmU16
            | Self::ImmI16
            | Self::ImmU32
            | Self::ImmI32
            | Self::ImmU64
            | Self::ImmI64
            | Self::ImmFelt
            | Self::ImmF64
            | Self::GlobalValue
            | Self::Load
            | Self::PtrToInt
            | Self::IntToPtr
            | Self::Cast
            | Self::Trunc
            | Self::Zext
            | Self::Sext
            | Self::Test
            | Self::Select
            | Self::Add
            | Self::Sub
            | Self::Mul
            | Self::Div
            | Self::Mod
            | Self::DivMod
            | Self::Neg
            | Self::Inv
            | Self::Incr
            | Self::Pow2
            | Self::Exp
            | Self::Not
            | Self::Bnot
            | Self::And
            | Self::Band
            | Self::Or
            | Self::Bor
            | Self::Xor
            | Self::Bxor
            | Self::Shl
            | Self::Shr
            | Self::Rotl
            | Self::Rotr
            | Self::Popcnt
            | Self::Eq
            | Self::Neq
            | Self::Gt
            | Self::Gte
            | Self::Lt
            | Self::Lte
            | Self::IsOdd
            | Self::Min
            | Self::Max => false,
        }
    }

    pub fn num_fixed_args(&self) -> usize {
        match self {
            Self::Assert | Self::Assertz => 1,
            Self::AssertEq => 2,
            // Immediates/constants have none
            Self::ImmI1
            | Self::ImmU8
            | Self::ImmI8
            | Self::ImmU16
            | Self::ImmI16
            | Self::ImmU32
            | Self::ImmI32
            | Self::ImmU64
            | Self::ImmI64
            | Self::ImmFelt
            | Self::ImmF64 => 0,
            // Binary ops always have two
            Self::Store
            | Self::Add
            | Self::Sub
            | Self::Mul
            | Self::Div
            | Self::Mod
            | Self::DivMod
            | Self::Exp
            | Self::And
            | Self::Band
            | Self::Or
            | Self::Bor
            | Self::Xor
            | Self::Bxor
            | Self::Shl
            | Self::Shr
            | Self::Rotl
            | Self::Rotr
            | Self::Eq
            | Self::Neq
            | Self::Gt
            | Self::Gte
            | Self::Lt
            | Self::Lte
            | Self::Min
            | Self::Max => 2,
            // Unary ops always have one
            Self::MemGrow
            | Self::Load
            | Self::PtrToInt
            | Self::IntToPtr
            | Self::Cast
            | Self::Trunc
            | Self::Zext
            | Self::Sext
            | Self::Test
            | Self::Neg
            | Self::Inv
            | Self::Incr
            | Self::Pow2
            | Self::Popcnt
            | Self::Not
            | Self::Bnot
            | Self::IsOdd => 1,
            // Select requires condition, arg1, and arg2
            Self::Select => 3,
            // MemCpy requires source, destination, and arity
            Self::MemCpy => 3,
            // Calls are entirely variable
            Self::Call | Self::Syscall => 0,
            // Unconditional branches have no fixed arguments
            Self::Br => 0,
            // Ifs have a single argument, the conditional
            Self::CondBr => 1,
            // Switches have a single argument, the input value
            Self::Switch => 1,
            // Returns require at least one argument
            Self::Ret => 1,
            // The following require no arguments
            Self::GlobalValue | Self::Alloca | Self::Unreachable | Self::InlineAsm => 0,
        }
    }

    pub(super) fn results(&self, ctrl_ty: Type) -> SmallVec<[Type; 1]> {
        use smallvec::smallvec;

        match self {
            // These ops have no results
            Self::Assert
            | Self::Assertz
            | Self::AssertEq
            | Self::Store
            | Self::MemGrow
            | Self::MemCpy
            | Self::Br
            | Self::CondBr
            | Self::Switch
            | Self::Ret
            | Self::Unreachable => smallvec![],
            // These ops have fixed result types
            Self::Test
            | Self::IsOdd
            | Self::Not
            | Self::And
            | Self::Or
            | Self::Xor
            | Self::Eq
            | Self::Neq
            | Self::Gt
            | Self::Gte
            | Self::Lt
            | Self::Lte => smallvec![Type::I1],
            // For these ops, the controlling type variable determines the type for the op
            Self::ImmI1
            | Self::ImmU8
            | Self::ImmI8
            | Self::ImmU16
            | Self::ImmI16
            | Self::ImmU32
            | Self::ImmI32
            | Self::ImmU64
            | Self::ImmI64
            | Self::ImmFelt
            | Self::ImmF64
            | Self::GlobalValue
            | Self::Alloca
            | Self::PtrToInt
            | Self::IntToPtr
            | Self::Cast
            | Self::Trunc
            | Self::Zext
            | Self::Sext
            | Self::Select
            | Self::Add
            | Self::Sub
            | Self::Mul
            | Self::Div
            | Self::Min
            | Self::Max
            | Self::Neg
            | Self::Inv
            | Self::Incr
            | Self::Pow2
            | Self::Popcnt
            | Self::Mod
            | Self::DivMod
            | Self::Exp
            | Self::Bnot
            | Self::Band
            | Self::Bor
            | Self::Bxor
            | Self::Shl
            | Self::Shr
            | Self::Rotl
            | Self::Rotr => {
                smallvec![ctrl_ty]
            }
            // The result type of a load is derived from the pointee type
            Self::Load => {
                smallvec![ctrl_ty.pointee().expect("expected pointer type").clone()]
            }
            // Call results are handled separately
            Self::Call | Self::Syscall | Self::InlineAsm => unreachable!(),
        }
    }
}
impl fmt::Display for Opcode {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Assert => f.write_str("assert"),
            Self::Assertz => f.write_str("assertz"),
            Self::AssertEq => f.write_str("assert.eq"),
            Self::ImmI1 => f.write_str("const.i1"),
            Self::ImmU8 => f.write_str("const.u8"),
            Self::ImmI8 => f.write_str("const.i8"),
            Self::ImmU16 => f.write_str("const.u16"),
            Self::ImmI16 => f.write_str("const.i16"),
            Self::ImmU32 => f.write_str("const.u32"),
            Self::ImmI32 => f.write_str("const.i32"),
            Self::ImmU64 => f.write_str("const.u64"),
            Self::ImmI64 => f.write_str("const.i64"),
            Self::ImmFelt => f.write_str("const.felt"),
            Self::ImmF64 => f.write_str("const.f64"),
            Self::GlobalValue => f.write_str("global"),
            Self::Alloca => f.write_str("alloca"),
            Self::MemGrow => f.write_str("memory.grow"),
            Self::Load => f.write_str("load"),
            Self::Store => f.write_str("store"),
            Self::MemCpy => f.write_str("memcpy"),
            Self::PtrToInt => f.write_str("ptrtoint"),
            Self::IntToPtr => f.write_str("inttoptr"),
            Self::Cast => f.write_str("cast"),
            Self::Trunc => f.write_str("trunc"),
            Self::Zext => f.write_str("zext"),
            Self::Sext => f.write_str("sext"),
            Self::Br => f.write_str("br"),
            Self::CondBr => f.write_str("condbr"),
            Self::Switch => f.write_str("switch"),
            Self::Call => f.write_str("call"),
            Self::Syscall => f.write_str("syscall"),
            Self::Ret => f.write_str("ret"),
            Self::Test => f.write_str("test"),
            Self::Select => f.write_str("select"),
            Self::Add => f.write_str("add"),
            Self::Sub => f.write_str("sub"),
            Self::Mul => f.write_str("mul"),
            Self::Div => f.write_str("div"),
            Self::Mod => f.write_str("mod"),
            Self::DivMod => f.write_str("divmod"),
            Self::Exp => f.write_str("exp"),
            Self::Neg => f.write_str("neg"),
            Self::Inv => f.write_str("inv"),
            Self::Incr => f.write_str("incr"),
            Self::Pow2 => f.write_str("pow2"),
            Self::Not => f.write_str("not"),
            Self::Bnot => f.write_str("bnot"),
            Self::And => f.write_str("and"),
            Self::Band => f.write_str("band"),
            Self::Or => f.write_str("or"),
            Self::Bor => f.write_str("bor"),
            Self::Xor => f.write_str("xor"),
            Self::Bxor => f.write_str("bxor"),
            Self::Shl => f.write_str("shl"),
            Self::Shr => f.write_str("shr"),
            Self::Rotl => f.write_str("rotl"),
            Self::Rotr => f.write_str("rotr"),
            Self::Popcnt => f.write_str("popcnt"),
            Self::Eq => f.write_str("eq"),
            Self::Neq => f.write_str("neq"),
            Self::Gt => f.write_str("gt"),
            Self::Gte => f.write_str("gte"),
            Self::Lt => f.write_str("lt"),
            Self::Lte => f.write_str("lte"),
            Self::IsOdd => f.write_str("is_odd"),
            Self::Min => f.write_str("min"),
            Self::Max => f.write_str("max"),
            Self::Unreachable => f.write_str("unreachable"),
            Self::InlineAsm => f.write_str("asm"),
        }
    }
}

/// This enumeration represents the various ways in which arithmetic operations
/// can be configured to behave when either the operands or results over/underflow
/// the range of the integral type.
///
/// Always check the documentation of the specific instruction involved to see if there
/// are any specific differences in how this enum is interpreted compared to the default
/// meaning of each variant.
#[derive(Copy, Clone, Default, Debug, PartialEq, Eq)]
pub enum Overflow {
    /// Typically, this means the operation is performed using the equivalent field element operation, rather
    /// than a dedicated operation for the given type. Because of this, the result of the operation may exceed
    /// that of the integral type expected, but this will not be caught right away.
    ///
    /// It is the callers responsibility to ensure that resulting value is in range.
    #[default]
    Unchecked,
    /// The operation will trap if the operands, or the result, is not valid for the range of the integral
    /// type involved, e.g. u32.
    Checked,
    /// The operation will wrap around, depending on the range of the integral type. For example,
    /// given a u32 value, this is done by applying `mod 2^32` to the result.
    Wrapping,
    /// The result of the operation will be computed as in [Wrapping], however in addition to the
    /// result, this variant also pushes a value on the stack which represents whether or not the
    /// operation over/underflowed; either 1 if over/underflow occurred, or 0 otherwise.
    Overflowing,
}
impl Overflow {
    /// Returns true if overflow is unchecked
    pub fn is_unchecked(&self) -> bool {
        matches!(self, Self::Unchecked)
    }

    /// Returns true if overflow will cause a trap
    pub fn is_checked(&self) -> bool {
        matches!(self, Self::Checked)
    }

    /// Returns true if overflow will add an extra boolean on top of the stack
    pub fn is_overflowing(&self) -> bool {
        matches!(self, Self::Overflowing)
    }
}

#[derive(Debug, Clone)]
pub struct GlobalValueOp {
    pub op: Opcode,
    pub global: GlobalValue,
}

#[derive(Debug, Clone)]
pub struct BinaryOp {
    pub op: Opcode,
    pub overflow: Overflow,
    pub args: [Value; 2],
}

#[derive(Debug, Clone)]
pub struct BinaryOpImm {
    pub op: Opcode,
    pub overflow: Overflow,
    pub arg: Value,
    pub imm: Immediate,
}

#[derive(Debug, Clone)]
pub struct UnaryOp {
    pub op: Opcode,
    pub overflow: Overflow,
    pub arg: Value,
}

#[derive(Debug, Clone)]
pub struct UnaryOpImm {
    pub op: Opcode,
    pub overflow: Overflow,
    pub imm: Immediate,
}

#[derive(Debug, Clone)]
pub struct Call {
    pub op: Opcode,
    pub callee: FunctionIdent,
    pub args: ValueList,
}

/// Branch
#[derive(Debug, Clone)]
pub struct Br {
    pub op: Opcode,
    pub destination: Block,
    pub args: ValueList,
}

/// Conditional Branch
#[derive(Debug, Clone)]
pub struct CondBr {
    pub op: Opcode,
    pub cond: Value,
    pub then_dest: (Block, ValueList),
    pub else_dest: (Block, ValueList),
}

/// Multi-way Branch w/Selector
#[derive(Debug, Clone)]
pub struct Switch {
    pub op: Opcode,
    pub arg: Value,
    pub arms: Vec<(u32, Block)>,
    pub default: Block,
}

/// Return
#[derive(Debug, Clone)]
pub struct Ret {
    pub op: Opcode,
    pub args: ValueList,
}

/// Return an immediate
#[derive(Debug, Clone)]
pub struct RetImm {
    pub op: Opcode,
    pub arg: Immediate,
}

/// Test
#[derive(Debug, Clone)]
pub struct Test {
    pub op: Opcode,
    pub arg: Value,
    pub ty: Type,
}

/// Load a value of type `ty` from `addr`
#[derive(Debug, Clone)]
pub struct LoadOp {
    pub op: Opcode,
    pub addr: Value,
    pub ty: Type,
}

/// A primop/intrinsic that takes a variable number of arguments
#[derive(Debug, Clone)]
pub struct PrimOp {
    pub op: Opcode,
    pub args: ValueList,
}

/// A primop that takes an immediate for its first argument, followed by a variable number of
/// arguments
#[derive(Debug, Clone)]
pub struct PrimOpImm {
    pub op: Opcode,
    pub imm: Immediate,
    pub args: ValueList,
}
