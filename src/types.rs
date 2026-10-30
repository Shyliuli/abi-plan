//! Target-independent C ABI types and call plans.
//!
//! The model separates two things that are often accidentally mixed:
//! `AbiType` describes the source object's layout, while `CallPlan` describes
//! how a call transports pieces of that object through registers and memory.
//! A backend or simulator executes the plan; this crate never emits machine
//! instructions.
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Endian {
    Little,
    Big,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DataLayout {
    pub pointer_bits: u16,
    pub pointer_align: u16,
    pub stack_align: u16,
    pub endian: Endian,
}

impl DataLayout {
    pub const SHY: Self = Self {
        pointer_bits: 32,
        pointer_align: 4,
        stack_align: 4,
        endian: Endian::Big,
    };
    pub const LP64: Self = Self {
        pointer_bits: 64,
        pointer_align: 8,
        stack_align: 16,
        endian: Endian::Little,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AbiKind {
    ShyV1,
    SysV64,
    WindowsX64,
    Aapcs64,
    Riscv64,
    LoongArch64,
}

impl AbiKind {
    pub fn data_layout(self) -> DataLayout {
        match self {
            AbiKind::ShyV1 => DataLayout::SHY,
            _ => DataLayout::LP64,
        }
    }
}

/// All C value shapes needed by the planners. Struct field offsets are already
/// source-language layout decisions; ABI classifiers must not silently relayout
/// them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AbiType {
    Void,
    Bool,
    Integer {
        bits: u16,
        signed: bool,
    },
    Float {
        bits: u16,
    },
    Pointer {
        bits: u16,
    },
    Array {
        element: Box<AbiType>,
        count: u32,
    },
    Struct {
        fields: Vec<AbiField>,
        size: u32,
        align: u16,
    },
    Union {
        fields: Vec<AbiField>,
        size: u32,
        align: u16,
    },
    Vector {
        element: Box<AbiType>,
        count: u32,
        size: u32,
        align: u16,
    },
    Complex {
        component: Box<AbiType>,
        size: u32,
        align: u16,
    },
    Opaque {
        size: u32,
        align: u16,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AbiField {
    pub name: Option<String>,
    pub ty: AbiType,
    pub offset: u32,
}

impl AbiField {
    pub fn new(name: impl Into<String>, ty: AbiType, offset: u32) -> Self {
        Self {
            name: Some(name.into()),
            ty,
            offset,
        }
    }
    pub fn anonymous(ty: AbiType, offset: u32) -> Self {
        Self {
            name: None,
            ty,
            offset,
        }
    }
}

impl AbiType {
    pub fn integer(bits: u16, signed: bool) -> Self {
        Self::Integer { bits, signed }
    }
    pub fn pointer(bits: u16) -> Self {
        Self::Pointer { bits }
    }
    pub fn float(bits: u16) -> Self {
        Self::Float { bits }
    }
    pub fn struct_type(fields: Vec<AbiField>, size: u32, align: u16) -> Self {
        Self::Struct {
            fields,
            size,
            align,
        }
    }
    pub fn union_type(fields: Vec<AbiField>, size: u32, align: u16) -> Self {
        Self::Union {
            fields,
            size,
            align,
        }
    }

    pub fn size(&self) -> u32 {
        match self {
            Self::Void => 0,
            Self::Bool => 1,
            Self::Integer { bits, .. } | Self::Float { bits } | Self::Pointer { bits } => {
                (*bits as u32).div_ceil(8)
            }
            Self::Array { element, count } => element.size().saturating_mul(*count),
            Self::Struct { size, .. }
            | Self::Union { size, .. }
            | Self::Vector { size, .. }
            | Self::Complex { size, .. }
            | Self::Opaque { size, .. } => *size,
        }
    }

    pub fn align(&self) -> u16 {
        match self {
            Self::Void => 1,
            Self::Bool => 1,
            Self::Integer { bits, .. } | Self::Float { bits } | Self::Pointer { bits } => {
                ((*bits as u32).div_ceil(8)).min(u16::MAX as u32) as u16
            }
            Self::Array { element, .. } => element.align(),
            Self::Struct { align, .. }
            | Self::Union { align, .. }
            | Self::Vector { align, .. }
            | Self::Complex { align, .. }
            | Self::Opaque { align, .. } => *align,
        }
    }

    pub fn is_void(&self) -> bool {
        matches!(self, Self::Void)
    }
    pub fn is_integer(&self) -> bool {
        matches!(self, Self::Bool | Self::Integer { .. })
    }
    pub fn is_float(&self) -> bool {
        matches!(self, Self::Float { .. })
    }
    pub fn is_pointer(&self) -> bool {
        matches!(self, Self::Pointer { .. })
    }
    pub fn is_aggregate(&self) -> bool {
        matches!(
            self,
            Self::Array { .. } | Self::Struct { .. } | Self::Union { .. }
        )
    }
    pub fn is_memory_only(&self) -> bool {
        self.is_aggregate() || matches!(self, Self::Opaque { .. })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionSignature {
    pub ret: AbiType,
    pub params: Vec<AbiType>,
    pub variadic: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RegisterClass {
    Gpr,
    Fpr,
    Vector,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RegisterId {
    pub class: RegisterClass,
    pub index: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Location {
    Register(RegisterId),
    Stack { offset: i32 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValuePart {
    pub source_offset: u32,
    pub size: u32,
    pub location: Location,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Passing {
    Direct {
        parts: Vec<ValuePart>,
    },
    Indirect {
        pointer: Location,
        size: u32,
        align: u16,
        byval: bool,
    },
    Ignore,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArgPlan {
    pub index: usize,
    pub ty: AbiType,
    pub passing: Passing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReturnPlan {
    Void,
    Direct {
        parts: Vec<ValuePart>,
    },
    Indirect {
        pointer: Location,
        size: u32,
        align: u16,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegisterUse {
    pub register: RegisterId,
    pub source: Option<(usize, u32, u32)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StackPlan {
    pub argument_bytes: u32,
    pub alignment: u16,
    pub shadow_bytes: u32,
    pub return_address_bytes: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallPlan {
    pub abi: AbiKind,
    pub signature: FunctionSignature,
    pub args: Vec<ArgPlan>,
    pub return_value: ReturnPlan,
    pub gpr: Vec<RegisterUse>,
    pub fpr: Vec<RegisterUse>,
    pub vector: Vec<RegisterUse>,
    pub stack: StackPlan,
}

impl CallPlan {
    pub fn stack_bytes(&self) -> u32 {
        self.stack.argument_bytes + self.stack.shadow_bytes
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AbiError {
    Unsupported { abi: AbiKind, message: String },
    InvalidType(String),
    InvalidLayout(String),
    TooManyArguments { abi: AbiKind, index: usize },
    SignatureConflict(String),
}

impl fmt::Display for AbiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported { abi, message } => write!(f, "{abi:?}: {message}"),
            Self::InvalidType(message)
            | Self::InvalidLayout(message)
            | Self::SignatureConflict(message) => f.write_str(message),
            Self::TooManyArguments { abi, index } => {
                write!(f, "{abi:?}: argument {index} cannot be represented")
            }
        }
    }
}

impl std::error::Error for AbiError {}
