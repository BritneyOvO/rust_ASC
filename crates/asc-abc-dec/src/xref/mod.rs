//! Instruction references and conservative value flow, independent of AST optimization.

mod constants;
mod flow;
mod index;

use std::collections::BTreeSet;

use regex::Regex;

use crate::{Error, Result};
use constants::integer;
use flow::{Edge, Flow};
use constants::Site;
use index::Index;

#[derive(Clone, Copy, Debug)]
pub enum RefQuery<'a> {
    String(&'a str),
    Constant(&'a str),
    Integer(&'a str),
    /// A regular expression over one resolved Ark instruction, including operands.
    Code(&'a str),
    Type(&'a str),
    Method {
        name: Option<&'a str>,
        class_name: Option<&'a str>,
    },
    Field {
        name: Option<&'a str>,
        class_name: Option<&'a str>,
    },
}

/// A bytecode or metadata hit; pc always refers to Ark bytecode, never generated source.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ReferenceHit {
    pub member: String,
    pub caller: String,
    pub pc: Option<u32>,
    pub opcode: Option<&'static str>,
    pub kind: &'static str,
    pub resolution: Option<&'static str>,
    pub matched: String,
    pub literal: Option<(u32, usize)>,
    pub metadata: Option<u32>,
}

impl ReferenceHit {
    fn definition(member: &str, caller: String, matched: String) -> Self {
        Self { member: member.into(), caller, pc: None, opcode: None, kind: "definition",
            resolution: None, matched, literal: None, metadata: None }
    }
}

impl std::fmt::Display for ReferenceHit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} | {}", self.member, self.caller)?;
        if let Some(pc) = self.pc { write!(f, " | pc={pc:#x}")?; }
        if let Some(opcode) = self.opcode { write!(f, " | opcode={opcode}")?; }
        if let Some((offset, index)) = self.literal { write!(f, " | literal@{offset:#x}[{index}]")?; }
        if let Some(offset) = self.metadata { write!(f, " | metadata@{offset:#x}")?; }
        write!(f, " | kind={}", self.kind)?;
        if let Some(resolution) = self.resolution { write!(f, " | resolution={resolution}")?; }
        write!(f, " | matched=({})", escaped(&self.matched))
    }
}

type Resolve<'a> = dyn Fn(&[u8]) -> Option<String> + 'a;

/// Reuse the parsed instructions and lazily built caller graph across queries.
pub struct References<'a> {
    index: Index<'a>,
    edges: Option<BTreeSet<Edge>>,
}

pub fn find_refs(data: &[u8], file: &str, query: RefQuery<'_>) -> Result<Vec<String>> {
    References::new(data)?.find(file, query)
}

pub fn find_refs_resolved(
    data: &[u8],
    file: &str,
    query: RefQuery<'_>,
    resolve: &Resolve<'_>,
) -> Result<Vec<String>> {
    References::new(data)?.find_resolved(file, query, resolve)
}

impl<'a> References<'a> {
    pub fn new(data: &'a [u8]) -> Result<Self> {
        Ok(Self {
            index: Index::new(data)?,
            edges: None,
        })
    }
    pub fn find(&mut self, file: &str, query: RefQuery<'_>) -> Result<Vec<String>> {
        self.search(file, query, None).map(|hits| hits.into_iter().map(|hit| hit.to_string()).collect())
    }
    pub fn find_resolved(
        &mut self,
        file: &str,
        query: RefQuery<'_>,
        resolve: &Resolve<'_>,
    ) -> Result<Vec<String>> {
        self.search(file, query, Some(resolve)).map(|hits| hits.into_iter().map(|hit| hit.to_string()).collect())
    }
    pub fn find_records(&mut self, file: &str, query: RefQuery<'_>) -> Result<Vec<ReferenceHit>> {
        self.search(file, query, None)
    }
    pub fn find_records_resolved(&mut self, file: &str, query: RefQuery<'_>, resolve: &Resolve<'_>) -> Result<Vec<ReferenceHit>> {
        self.search(file, query, Some(resolve))
    }
    fn search(
        &mut self,
        file: &str,
        query: RefQuery<'_>,
        resolve: Option<&Resolve<'_>>,
    ) -> Result<Vec<ReferenceHit>> {
        let mut lines = BTreeSet::new();
        match query {
            RefQuery::String(pattern)
            | RefQuery::Constant(pattern)
            | RefQuery::Integer(pattern) => {
                let exact = if matches!(query, RefQuery::Integer(_)) {
                    Some(integer(pattern).ok_or_else(|| {
                        Error::invalid("integer query requires a decimal or hexadecimal integer")
                    })?)
                } else {
                    None
                };
                let regex = if exact.is_none() {
                    Some(regex(pattern)?)
                } else {
                    None
                };
                for value in &self.index.constants {
                    if matches!(query, RefQuery::String(_)) && !value.kind.is_string() {
                        continue;
                    }
                    let resolved = if value.kind.is_string() {
                        resolve.and_then(|f| f(value.text.as_bytes()))
                    } else {
                        None
                    };
                    let hit = if let Some(exact) = &exact {
                        value.kind.is_integer() && integer(&value.text).as_ref() == Some(exact)
                    } else {
                        let regex = regex.as_ref().expect("noninteger query compiles a regex");
                        regex.is_match(&value.text)
                            || resolved.as_ref().is_some_and(|text| regex.is_match(text))
                    };
                    if hit {
                        let text = resolved.as_deref().unwrap_or(&value.text);
                        let (caller, pc, opcode, literal, metadata) = match &value.site {
                            Site::Instruction { method, pc, opcode, literal } => (self.index.method_name(*method), Some(*pc), Some(*opcode), *literal, None),
                            Site::Literal { offset, index } => (format!("literal@{offset:#x}[{index}]"), None, None, None, None),
                            Site::Metadata { owner, offset } => (owner.clone(), None, None, None, Some(*offset)),
                        };
                        lines.insert(ReferenceHit { member: file.into(), caller, pc, opcode, literal, metadata,
                            kind: value.kind.name(), resolution: None, matched: text.into() });
                    }
                }
            }
            RefQuery::Code(pattern) => {
                let regex = regex(pattern)?;
                for (&offset, method) in &self.index.methods {
                    for ins in &method.instructions {
                        let code = self.index.assembly(ins)?;
                        if regex.is_match(&code) {
                            lines.insert(ReferenceHit { member: file.into(), caller: self.index.method_name(offset),
                                pc: Some(ins.ins.pc), opcode: Some(ins.ins.opcode.name), kind: "code", resolution: None,
                                matched: code, literal: None, metadata: None });
                        }
                    }
                }
            }
            RefQuery::Type(_) | RefQuery::Method { .. } | RefQuery::Field { .. } => {
                let matcher = Matcher::new(query)?;
                if self.edges.is_none() {
                    self.edges = Some(Flow::new(&self.index).run()?);
                }
                for edge in self.edges.as_ref().expect("caller graph initialized") {
                    let target = self.index.target_name(&edge.target);
                    if matcher.matches(&self.index, &edge.target, edge.kind) {
                        lines.insert(ReferenceHit { member: file.into(), caller: self.index.method_name(edge.caller),
                            pc: Some(edge.pc), opcode: Some(edge.opcode), kind: edge.kind.name(),
                            resolution: Some(edge.target.resolution()), matched: target, literal: None, metadata: None });
                    }
                }
                // Definitions remain searchable, but are explicitly distinguished from use sites.
                self.index.definitions(file, &matcher, &mut lines);
            }
        }
        Ok(lines.into_iter().collect())
    }
}

fn regex(pattern: &str) -> Result<Regex> {
    Regex::new(pattern).map_err(|err| Error::invalid(format!("invalid search regex: {err}")))
}

fn escaped(text: &str) -> String {
    let mut output = String::new();
    for c in text.chars() {
        match c {
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            '\0' => output.push_str("\\0"),
            c => output.push(c),
        }
    }
    output
}

struct Matcher {
    query: QueryKind,
    name: Regex,
    class: Option<String>,
}

#[derive(Clone, Copy)]
enum QueryKind {
    Type,
    Method,
    Field,
}

impl Matcher {
    fn new(query: RefQuery<'_>) -> Result<Self> {
        let (query, pattern, class) = match query {
            RefQuery::Type(pattern) => (QueryKind::Type, pattern, None),
            RefQuery::Method { name, class_name } => {
                (QueryKind::Method, name.unwrap_or(".*"), class_name)
            }
            RefQuery::Field { name, class_name } => {
                (QueryKind::Field, name.unwrap_or(".*"), class_name)
            }
            RefQuery::String(_)
            | RefQuery::Constant(_)
            | RefQuery::Integer(_)
            | RefQuery::Code(_) => return Err(Error::invalid("expected a member or type query")),
        };
        let pattern = if matches!(query, QueryKind::Type) {
            pattern.to_string()
        } else {
            format!("^(?:{pattern})$")
        };
        Ok(Self {
            query,
            name: regex(&pattern)?,
            class: class.map(str::to_owned),
        })
    }
}

#[cfg(test)]
mod tests;
