use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::decode::{decode, Instruction, Operand, FIELD_ID, TYPE_ID};
use crate::file::{Class, Field, FieldValue, File, Literal, Method, Reader};
use crate::{Error, Result, SizeBudget};

use super::constants::{self, Constant, ConstantKind, Site};
use super::flow::{EdgeKind, Target};
use super::{Matcher, QueryKind, ReferenceHit};

pub(super) fn base(name: &str) -> &str {
    name.strip_prefix("wide.")
        .or_else(|| name.strip_prefix("deprecated."))
        .unwrap_or(name)
}

#[derive(Clone, Copy, Debug)]
pub(super) enum IdKind {
    String,
    Method,
    Literal,
    Type,
    Field,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Reference {
    pub operand: usize,
    pub kind: IdKind,
    pub offset: u32,
}

pub(super) struct Resolved {
    pub ins: Instruction,
    pub references: Vec<Reference>,
}

impl Resolved {
    pub fn string(&self) -> Option<u32> {
        self.references
            .iter()
            .find_map(|r| matches!(r.kind, IdKind::String).then_some(r.offset))
    }
    pub fn method(&self) -> Option<u32> {
        self.references
            .iter()
            .find_map(|r| matches!(r.kind, IdKind::Method).then_some(r.offset))
    }
    pub fn literal(&self) -> Option<u32> {
        self.references
            .iter()
            .find_map(|r| matches!(r.kind, IdKind::Literal).then_some(r.offset))
    }
    pub fn reg(&self, index: usize) -> Result<u32> {
        self.ins
            .operands
            .iter()
            .filter_map(|op| {
                if let Operand::Register(r) = op {
                    Some(*r)
                } else {
                    None
                }
            })
            .nth(index)
            .ok_or_else(|| Error::invalid("missing xref register operand"))
    }
    pub fn imms(&self) -> Vec<i64> {
        self.ins
            .operands
            .iter()
            .filter_map(|op| {
                if let Operand::Immediate(v) = op {
                    Some(*v)
                } else {
                    None
                }
            })
            .collect()
    }
}

pub(super) struct MethodInfo<'a> {
    pub record: usize,
    pub method: Method<'a>,
    pub instructions: Vec<Resolved>,
}

pub(super) struct ClassInfo {
    pub record: usize,
    pub name: String,
    /// Instance/static member, property name, method offset and literal tag.
    pub members: BTreeMap<(bool, String), Vec<(u32, u8)>>,
}

pub(super) struct Owner {
    pub class: u32,
    pub static_: bool,
    pub name: String,
}

pub(super) struct Index<'a> {
    pub file: File<'a>,
    pub records: Vec<Class<'a>>,
    pub methods: BTreeMap<u32, MethodInfo<'a>>,
    pub strings: BTreeMap<u32, String>,
    pub literals: BTreeMap<u32, Vec<(u8, Literal)>>,
    pub classes: BTreeMap<u32, ClassInfo>,
    pub owners: BTreeMap<u32, Owner>,
    pub constants: Vec<Constant>,
    pub fields: Vec<(usize, Field)>,
}

impl<'a> Index<'a> {
    pub fn new(data: &'a [u8]) -> Result<Self> {
        let file = File::parse(data)?;
        let mut index = Self {
            file,
            records: Vec::new(),
            methods: BTreeMap::new(),
            strings: BTreeMap::new(),
            literals: BTreeMap::new(),
            classes: BTreeMap::new(),
            owners: BTreeMap::new(),
            constants: Vec::new(),
            fields: Vec::new(),
        };
        let mut pending = VecDeque::new();
        let mut budget = SizeBudget::new("xref index");
        for off in index.file.class_offsets()? {
            let (mut class, fields) = index.file.class_with_fields(off)?;
            let record = index.records.len();
            index.metadata(class.name.clone(), off, class.name.clone());
            for field in fields {
                let owner = format!("{}->{}", class.name, field.name);
                index.metadata(owner.clone(), field.offset, field.name.clone());
                if let Some(value) = &field.value {
                    let (kind, text) = match value {
                        FieldValue::Integer(v) => (ConstantKind::Integer, v.to_string()),
                        FieldValue::Offset(offset) => {
                            let ty = index.file.reference(
                                field.offset,
                                u32::from(field.type_index),
                                TYPE_ID,
                            )?;
                            let mut reader = Reader::new(index.file.data(), *offset)?;
                            match ty {
                                5 => (
                                    ConstantKind::Integer,
                                    i32::from_le_bytes(reader.u32()?.to_le_bytes()).to_string(),
                                ),
                                6 => (ConstantKind::Integer, reader.u32()?.to_string()),
                                7 => (
                                    ConstantKind::Float,
                                    crate::file::number(f64::from(f32::from_bits(reader.u32()?))),
                                ),
                                8 => (
                                    ConstantKind::Float,
                                    crate::file::number(f64::from_bits(reader.u64()?)),
                                ),
                                9 => (
                                    ConstantKind::Integer,
                                    i64::from_le_bytes(reader.u64()?.to_le_bytes()).to_string(),
                                ),
                                10 => (ConstantKind::Integer, reader.u64()?.to_string()),
                                _ => {
                                    index.fields.push((record, field));
                                    continue;
                                }
                            }
                        }
                    };
                    index.constants.push(Constant {
                        site: Site::Metadata {
                            owner,
                            offset: field.offset,
                        },
                        kind,
                        text,
                    });
                }
                index.fields.push((record, field));
            }
            for method in std::mem::take(&mut class.methods) {
                pending.push_back((record, method));
            }
            for request in &class.module.requests {
                index.metadata(class.name.clone(), off, request.clone());
            }
            for import in &class.module.imports {
                index.metadata(class.name.clone(), off, import.local.clone());
                if let Some(name) = &import.imported {
                    index.metadata(class.name.clone(), off, name.clone());
                }
            }
            for export in &class.module.exports {
                if let Some(name) = &export.local {
                    index.metadata(class.name.clone(), off, name.clone());
                }
                if let Some(name) = &export.exported {
                    index.metadata(class.name.clone(), off, name.clone());
                }
            }
            index.records.push(class);
        }
        while let Some((record, method)) = pending.pop_front() {
            if index.methods.contains_key(&method.offset) {
                continue;
            }
            budget.charge(method.name.len() + std::mem::size_of::<MethodInfo>())?;
            index.metadata(
                format!("{}->{}", index.records[record].name, method.name),
                method.offset,
                method.name.clone(),
            );
            let mut instructions = Vec::new();
            if let Some(code) = &method.code {
                for ins in decode(code.bytes)? {
                    budget.charge(
                        std::mem::size_of::<Resolved>()
                            + ins.operands.len() * std::mem::size_of::<Operand>(),
                    )?;
                    let mut references = Vec::new();
                    for (operand, value) in ins.operands.iter().enumerate() {
                        let Operand::Id(id) = value else {
                            continue;
                        };
                        let flags = ins.opcode.flags;
                        let kind = if flags & TYPE_ID != 0 {
                            IdKind::Type
                        } else if flags & 2 != 0 && references.is_empty() {
                            IdKind::Method
                        } else if flags & 4 != 0 {
                            IdKind::String
                        } else if flags & 8 != 0 {
                            IdKind::Literal
                        } else if flags & FIELD_ID != 0 {
                            IdKind::Field
                        } else {
                            return Err(Error::invalid("instruction ID has no semantic kind"));
                        };
                        let offset = index.file.reference(
                            method.offset,
                            *id,
                            flags & (TYPE_ID | FIELD_ID),
                        )?;
                        match kind {
                            IdKind::String => {
                                if !index.strings.contains_key(&offset) {
                                    index.strings.insert(offset, index.file.string(offset)?);
                                }
                                let kind = match base(ins.opcode.name) {
                                    "ldbigint" => ConstantKind::BigInt,
                                    "createregexpwithliteral" => ConstantKind::RegExp,
                                    _ => ConstantKind::String,
                                };
                                index.constants.push(Constant::instruction(
                                    method.offset,
                                    &ins,
                                    kind,
                                    index.strings[&offset].clone(),
                                ));
                            }
                            IdKind::Literal => {
                                index.load_literals(offset, &mut BTreeSet::new())?;
                                index.literal_constants(
                                    offset,
                                    Site::Instruction {
                                        method: method.offset,
                                        pc: ins.pc,
                                        opcode: ins.opcode.name,
                                        literal: None,
                                    },
                                    &mut BTreeSet::new(),
                                )?;
                                for target in index.literal_methods(offset, &mut BTreeSet::new()) {
                                    if !index.methods.contains_key(&target) {
                                        pending.push_back((record, index.file.method(target)?));
                                    }
                                }
                            }
                            IdKind::Method => {
                                if !index.methods.contains_key(&offset) {
                                    pending.push_back((record, index.file.method(offset)?));
                                }
                            }
                            IdKind::Type | IdKind::Field => {}
                        }
                        references.push(Reference {
                            operand,
                            kind,
                            offset,
                        });
                    }
                    if let Some(value) = constants::immediate(method.offset, &ins) {
                        index.constants.push(value);
                    }
                    instructions.push(Resolved { ins, references });
                }
            }
            index.methods.insert(
                method.offset,
                MethodInfo {
                    record,
                    method,
                    instructions,
                },
            );
        }
        // Older files also expose unused literal buffers through their header.
        if &index.file.data()[12..16] <= [12, 0, 6, 0].as_slice() {
            let mut header = Reader::new(index.file.data(), 44)?;
            let count = header.u32()?;
            let offset = header.u32()?;
            let mut reader = Reader::new(index.file.data(), offset)?;
            for _ in 0..count {
                let offset = reader.u32()?;
                index.load_literals(offset, &mut BTreeSet::new())?;
                index.literal_constants(
                    offset,
                    Site::Literal { offset, index: 0 },
                    &mut BTreeSet::new(),
                )?;
            }
        }
        index.class_shapes()?;
        Ok(index)
    }
    fn metadata(&mut self, owner: String, offset: u32, text: String) {
        self.constants.push(Constant {
            site: Site::Metadata { owner, offset },
            kind: ConstantKind::String,
            text,
        });
    }
    fn load_literals(&mut self, offset: u32, path: &mut BTreeSet<u32>) -> Result<()> {
        if !path.insert(offset) {
            return Err(Error::invalid("cyclic literal array"));
        }
        if path.len() > 64 {
            return Err(Error::invalid("literal nesting exceeds 64"));
        }
        if !self.literals.contains_key(&offset) {
            self.literals
                .insert(offset, self.file.literal_entries(offset)?);
        }
        let nested = self.literals[&offset]
            .iter()
            .filter_map(|(_, v)| {
                if let Literal::Array(off) = v {
                    Some(*off)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        for off in nested {
            self.load_literals(off, path)?;
        }
        path.remove(&offset);
        Ok(())
    }
    fn literal_constants(
        &mut self,
        offset: u32,
        site: Site,
        visited: &mut BTreeSet<u32>,
    ) -> Result<()> {
        if !visited.insert(offset) {
            return Ok(());
        }
        let mut nested = Vec::new();
        for (i, (tag, value)) in self.literals[&offset].iter().enumerate() {
            let text = match value {
                Literal::Value(text) | Literal::String(text) => text,
                Literal::Array(off) => {
                    nested.push(*off);
                    continue;
                }
                Literal::Method { .. } | Literal::Affiliate(_) => continue,
            };
            let Some(kind) = ConstantKind::literal(*tag) else {
                continue;
            };
            let site = match &site {
                Site::Instruction {
                    method, pc, opcode, ..
                } => Site::Instruction {
                    method: *method,
                    pc: *pc,
                    opcode,
                    literal: Some((offset, i)),
                },
                Site::Literal { .. } => Site::Literal { offset, index: i },
                Site::Metadata { .. } => {
                    return Err(Error::invalid("metadata is not a literal use site"))
                }
            };
            self.constants.push(Constant {
                site,
                kind,
                text: text.clone(),
            });
        }
        for nested in nested {
            self.literal_constants(nested, site.clone(), visited)?;
        }
        Ok(())
    }
    pub fn literal_methods(&self, offset: u32, visited: &mut BTreeSet<u32>) -> Vec<u32> {
        if !visited.insert(offset) {
            return Vec::new();
        }
        self.literals
            .get(&offset)
            .into_iter()
            .flatten()
            .flat_map(|(_, value)| match value {
                Literal::Method { offset, .. } => vec![*offset],
                Literal::Array(off) => self.literal_methods(*off, visited),
                Literal::Value(_) | Literal::String(_) | Literal::Affiliate(_) => Vec::new(),
            })
            .collect()
    }
    fn class_shapes(&mut self) -> Result<()> {
        for method in self.methods.values() {
            for ins in &method.instructions {
                if !matches!(
                    base(ins.ins.opcode.name),
                    "defineclasswithbuffer" | "callruntime.definesendableclass"
                ) {
                    continue;
                }
                let ctor = ins
                    .method()
                    .ok_or_else(|| Error::invalid("class has no constructor"))?;
                let buffer = ins
                    .literal()
                    .ok_or_else(|| Error::invalid("class has no literal buffer"))?;
                let values = &self.literals[&buffer];
                let trailer = if ins.ins.opcode.name == "callruntime.definesendableclass" {
                    2
                } else {
                    1
                };
                let count_index = values
                    .len()
                    .checked_sub(trailer)
                    .ok_or_else(|| Error::invalid("empty class method buffer"))?;
                let count = match &values[count_index].1 {
                    Literal::Value(count) => count
                        .parse::<usize>()
                        .map_err(|_| Error::invalid("invalid class instance count"))?,
                    _ => return Err(Error::invalid("class instance count is not numeric")),
                };
                if count_index % 3 != 0 || count > count_index / 3 {
                    return Err(Error::invalid("invalid class method buffer"));
                }
                let ctor_method = &self.methods[&ctor].method;
                let name = ctor_method
                    .name
                    .rsplit('#')
                    .next()
                    .unwrap_or(&ctor_method.name)
                    .to_string();
                let mut members: BTreeMap<_, Vec<_>> = BTreeMap::new();
                self.owners.insert(
                    ctor,
                    Owner {
                        class: ctor,
                        static_: false,
                        name: "constructor".into(),
                    },
                );
                for (i, group) in values[..count_index].as_chunks::<3>().0.iter().enumerate() {
                    let (Literal::String(name), Literal::Method { offset, tag }) =
                        (&group[0].1, &group[1].1)
                    else {
                        return Err(Error::invalid("invalid class member"));
                    };
                    let static_ = i >= count;
                    members
                        .entry((static_, name.clone()))
                        .or_default()
                        .push((*offset, *tag));
                    self.owners.insert(
                        *offset,
                        Owner {
                            class: ctor,
                            static_,
                            name: name.clone(),
                        },
                    );
                }
                self.classes.insert(
                    ctor,
                    ClassInfo {
                        record: method.record,
                        name,
                        members,
                    },
                );
            }
        }
        Ok(())
    }
    pub fn method_name(&self, offset: u32) -> String {
        let method = &self.methods[&offset];
        if let Some(owner) = self.owners.get(&offset) {
            format!(
                "{}::{}->{}@{offset:#x}",
                self.records[method.record].name, self.classes[&owner.class].name, owner.name
            )
        } else {
            format!(
                "{}->{}@{offset:#x}",
                self.records[method.record].name, method.method.name
            )
        }
    }
    pub fn target_name(&self, target: &Target) -> String {
        match target {
            Target::Method(off) => self.method_name(*off),
            Target::Class(off) => {
                let class = &self.classes[off];
                format!(
                    "{}::{}@{off:#x}",
                    self.records[class.record].name, class.name
                )
            }
            Target::Record(i) => self.records[*i].name.clone(),
            Target::Named { owner, name } => {
                if owner.is_empty() {
                    name.clone()
                } else {
                    format!("{owner}->{name}")
                }
            }
        }
    }
    pub fn assembly(&self, resolved: &Resolved) -> Result<String> {
        let mut parts = Vec::new();
        for (i, operand) in resolved.ins.operands.iter().enumerate() {
            parts.push(match operand {
                Operand::Register(v) => format!("v{v}"),
                Operand::Immediate(v) => v.to_string(),
                Operand::Float(v) => crate::file::number(*v),
                Operand::Id(_) => {
                    let r = resolved
                        .references
                        .iter()
                        .find(|r| r.operand == i)
                        .expect("every ID is resolved");
                    match r.kind {
                        IdKind::String => format!(
                            "string@{:#x} {}",
                            r.offset,
                            crate::lower::quote(&self.strings[&r.offset])
                        ),
                        IdKind::Method => {
                            format!("method@{:#x} {}", r.offset, self.method_name(r.offset))
                        }
                        IdKind::Literal => format!("literal@{:#x}", r.offset),
                        IdKind::Type => format!("type@{:#x}", r.offset),
                        IdKind::Field => format!("field@{:#x}", r.offset),
                    }
                }
            });
        }
        Ok(format!("{} {}", resolved.ins.opcode.name, parts.join(", ")))
    }
    pub fn definitions(&self, file: &str, matcher: &Matcher, lines: &mut BTreeSet<ReferenceHit>) {
        if matches!(matcher.query, QueryKind::Field) {
            for (record, field) in &self.fields {
                if matcher.name.is_match(&field.name)
                    && matcher
                        .class
                        .as_deref()
                        .is_none_or(|query| self.class_matches(*record, None, query))
                {
                    lines.insert(ReferenceHit::definition(file,
                        format!("{}->{}@{:#x}", self.records[*record].name, field.name, field.offset), field.name.clone()));
                }
            }
        }
        for &off in self.methods.keys() {
            let target = Target::Method(off);
            if matcher.matches(self, &target, EdgeKind::Function) {
                lines.insert(ReferenceHit::definition(file, self.method_name(off), self.method_name(off)));
            }
        }
        for (&off, class) in &self.classes {
            let target = Target::Class(off);
            if matcher.matches(self, &target, EdgeKind::Class) {
                lines.insert(ReferenceHit::definition(file, self.records[class.record].name.clone(), self.target_name(&target)));
            }
        }
        for record in 0..self.records.len() {
            let target = Target::Record(record);
            if matcher.matches(self, &target, EdgeKind::Class) {
                lines.insert(ReferenceHit::definition(file, self.records[record].name.clone(), self.records[record].name.clone()));
            }
        }
    }
    pub fn class_matches(&self, record: usize, class: Option<&str>, query: &str) -> bool {
        let normalize = |s: &str| {
            s.trim_start_matches('L')
                .trim_end_matches(';')
                .replace('.', "/")
        };
        query == self.records[record].name
            || normalize(query) == normalize(&self.records[record].name)
            || class.is_some_and(|name| name == query)
    }
}

impl Matcher {
    pub fn matches(&self, index: &Index<'_>, target: &Target, kind: EdgeKind) -> bool {
        let (record, class, name) = match target {
            Target::Method(off) => {
                let method = &index.methods[off];
                let owner = index.owners.get(off);
                let class = owner.map(|owner| index.classes[&owner.class].name.as_str());
                let name = owner.map_or_else(
                    || {
                        method
                            .method
                            .name
                            .rsplit('#')
                            .next()
                            .unwrap_or(&method.method.name)
                    },
                    |owner| owner.name.as_str(),
                );
                if matches!(self.query, QueryKind::Type) {
                    return self.name.is_match(&index.records[method.record].name)
                        || class.is_some_and(|c| self.name.is_match(c));
                }
                if matches!(self.query, QueryKind::Field) {
                    return false;
                }
                (Some(method.record), class, name)
            }
            Target::Class(off) => {
                let class = &index.classes[off];
                if matches!(self.query, QueryKind::Type) {
                    return self.name.is_match(&class.name)
                        || self.name.is_match(&index.records[class.record].name);
                }
                if !matches!(self.query, QueryKind::Method) || !matches!(kind, EdgeKind::Construct)
                {
                    return false;
                }
                (Some(class.record), Some(class.name.as_str()), "constructor")
            }
            Target::Record(record) => {
                return matches!(self.query, QueryKind::Type)
                    && self.name.is_match(&index.records[*record].name)
            }
            Target::Named { owner, name } => {
                if matches!(self.query, QueryKind::Type) {
                    return !owner.is_empty() && self.name.is_match(owner);
                }
                let accepted = match self.query {
                    QueryKind::Method => matches!(
                        kind,
                        EdgeKind::Call
                            | EdgeKind::Construct
                            | EdgeKind::Function
                            | EdgeKind::Getter
                            | EdgeKind::Setter
                    ),
                    QueryKind::Field => matches!(kind, EdgeKind::Read | EdgeKind::Write),
                    QueryKind::Type => false,
                };
                if !accepted {
                    return false;
                }
                (None, Some(owner.as_str()), name.as_str())
            }
        };
        if let Some(query) = &self.class {
            let matches = match record {
                Some(record) => index.class_matches(record, class, query),
                None => class
                    .is_some_and(|class| class == query || class.rsplit('.').next() == Some(query)),
            };
            if !matches {
                return false;
            }
        }
        self.name.is_match(name)
    }
}
