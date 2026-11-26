use crate::decode::{Instruction, Operand};

#[derive(Clone, Copy, Debug)]
pub(super) enum ConstantKind {
    String,
    Integer,
    Float,
    BigInt,
    Boolean,
    Null,
    Undefined,
    Hole,
    RegExp,
}

impl ConstantKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Integer => "integer",
            Self::Float => "float",
            Self::BigInt => "bigint",
            Self::Boolean => "boolean",
            Self::Null => "null",
            Self::Undefined => "undefined",
            Self::Hole => "hole",
            Self::RegExp => "regexp",
        }
    }
    pub fn is_string(self) -> bool {
        matches!(self, Self::String | Self::RegExp)
    }
    pub fn is_integer(self) -> bool {
        matches!(self, Self::Integer | Self::BigInt)
    }
    pub fn literal(tag: u8) -> Option<Self> {
        match tag {
            0x00 | 0x02 | 0x0b..=0x12 | 0x17 => Some(Self::Integer),
            0x01 | 0x0a => Some(Self::Boolean),
            0x03 | 0x04 | 0x13 | 0x14 => Some(Self::Float),
            0x05 | 0x15 => Some(Self::String),
            0x08 | 0x19 => Some(Self::Undefined),
            0xff => Some(Self::Null),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct Constant {
    pub site: Site,
    pub kind: ConstantKind,
    pub text: String,
}

#[derive(Clone, Debug)]
pub(super) enum Site {
    Instruction {
        method: u32,
        pc: u32,
        opcode: &'static str,
        literal: Option<(u32, usize)>,
    },
    Literal {
        offset: u32,
        index: usize,
    },
    Metadata {
        owner: String,
        offset: u32,
    },
}

impl Constant {
    pub fn instruction(method: u32, ins: &Instruction, kind: ConstantKind, text: String) -> Self {
        Self {
            site: Site::Instruction {
                method,
                pc: ins.pc,
                opcode: ins.opcode.name,
                literal: None,
            },
            kind,
            text,
        }
    }
}

/// Canonical decimal text, retaining arbitrary precision for BigInt and uint64 arrays.
pub(super) fn integer(text: &str) -> Option<String> {
    let text = text.strip_suffix('n').unwrap_or(text);
    let (negative, digits) = if let Some(text) = text.strip_prefix('-') {
        (true, text)
    } else {
        (false, text.strip_prefix('+').unwrap_or(text))
    };
    let decimal = if let Some(hex) = digits
        .strip_prefix("0x")
        .or_else(|| digits.strip_prefix("0X"))
    {
        if hex.is_empty() || !hex.bytes().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let mut decimal = vec![0u8];
        for digit in hex.bytes() {
            let mut carry = u16::from(match digit {
                b'0'..=b'9' => digit - b'0',
                b'a'..=b'f' => digit - b'a' + 10,
                b'A'..=b'F' => digit - b'A' + 10,
                _ => return None,
            });
            for value in &mut decimal {
                let next = u16::from(*value) * 16 + carry;
                *value = u8::try_from(next % 10).expect("decimal digit fits u8");
                carry = next / 10;
            }
            while carry != 0 {
                decimal.push(u8::try_from(carry % 10).expect("decimal digit fits u8"));
                carry /= 10;
            }
        }
        decimal
            .into_iter()
            .rev()
            .map(|c| char::from(b'0' + c))
            .collect::<String>()
    } else {
        if digits.is_empty() || !digits.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let digits = digits.trim_start_matches('0');
        if digits.is_empty() {
            "0".to_string()
        } else {
            digits.to_string()
        }
    };
    Some(if negative && decimal != "0" {
        format!("-{decimal}")
    } else {
        decimal
    })
}

pub(super) fn immediate(method: u32, ins: &Instruction) -> Option<Constant> {
    let name = super::index::base(ins.opcode.name);
    let (kind, text) = match name {
        "ldai" => (
            ConstantKind::Integer,
            match ins.operands.first()? {
                Operand::Immediate(v) => v.to_string(),
                _ => return None,
            },
        ),
        "fldai" => (
            ConstantKind::Float,
            match ins.operands.first()? {
                Operand::Float(v) => crate::file::number(*v),
                _ => return None,
            },
        ),
        "ldtrue" => (ConstantKind::Boolean, "true".into()),
        "ldfalse" => (ConstantKind::Boolean, "false".into()),
        "ldnull" => (ConstantKind::Null, "null".into()),
        "ldundefined" | "returnundefined" => (ConstantKind::Undefined, "undefined".into()),
        "ldhole" => (ConstantKind::Hole, "hole".into()),
        "ldnan" => (ConstantKind::Float, "NaN".into()),
        "ldinfinity" => (ConstantKind::Float, "Infinity".into()),
        _ => return None,
    };
    Some(Constant::instruction(method, ins, kind, text))
}
