use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::decode::{CONDITIONAL, JUMP};
use crate::file::Literal;
use crate::{Error, Result};

use super::index::{base, IdKind, Index, MethodInfo, Resolved};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Target {
    Method(u32),
    Class(u32),
    Record(usize),
    Named { owner: String, name: String },
}
impl Target {
    pub fn resolution(&self) -> &'static str {
        match self {
            Self::Method(_) | Self::Class(_) | Self::Record(_) => "resolved",
            Self::Named { .. } => "symbolic",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum EdgeKind {
    Call,
    Construct,
    Function,
    Class,
    Read,
    Write,
    Getter,
    Setter,
    Import,
}
impl EdgeKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Call => "call",
            Self::Construct => "construct",
            Self::Function => "function-ref",
            Self::Class => "class-ref",
            Self::Read => "read",
            Self::Write => "write",
            Self::Getter => "getter-call",
            Self::Setter => "setter-call",
            Self::Import => "import",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Edge {
    pub caller: u32,
    pub pc: u32,
    pub opcode: &'static str,
    pub kind: EdgeKind,
    pub target: Target,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Environment {
    method: u32,
    pc: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Atom {
    Function(u32),
    Class(u32),
    Instance(u32),
    Prototype(u32),
    Object { method: u32, pc: u32 },
    Symbol(String),
    String(String),
    Environment(Environment),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Value {
    atoms: BTreeSet<Atom>,
    unknown: bool,
}

impl Value {
    fn unknown() -> Self {
        Self {
            atoms: BTreeSet::new(),
            unknown: true,
        }
    }
    fn one(atom: Atom) -> Self {
        Self {
            atoms: [atom].into_iter().collect(),
            unknown: false,
        }
    }
    fn join(&mut self, other: &Self) -> Result<bool> {
        let old = self.atoms.len();
        let unknown = self.unknown;
        self.atoms.extend(other.atoms.iter().cloned());
        self.unknown |= other.unknown;
        if self.atoms.len() > 4096 {
            return Err(Error::invalid(
                "xref value has more than 4096 possible targets",
            ));
        }
        Ok(old != self.atoms.len() || unknown != self.unknown)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Binding {
    Global(usize, String),
    Module(usize, usize),
    Lexical(Environment, u32),
    Property(Atom, String),
    Return(u32),
    Argument(u32, u32),
    Capture(u32),
    Parent(Environment),
    Base(u32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct State {
    acc: Value,
    regs: BTreeMap<u32, Value>,
    env: Value,
    local: BTreeMap<Binding, Value>,
}

impl State {
    fn join(&mut self, other: &Self) -> Result<bool> {
        let mut changed = self.acc.join(&other.acc)? | self.env.join(&other.env)?;
        let keys: BTreeSet<_> = self.regs.keys().chain(other.regs.keys()).copied().collect();
        for key in keys {
            let value = other.regs.get(&key).cloned().unwrap_or_else(Value::unknown);
            changed |= self
                .regs
                .entry(key)
                .or_insert_with(Value::unknown)
                .join(&value)?;
        }
        let keys: BTreeSet<_> = self
            .local
            .keys()
            .chain(other.local.keys())
            .cloned()
            .collect();
        for key in keys {
            let value = other
                .local
                .get(&key)
                .cloned()
                .unwrap_or_else(Value::unknown);
            changed |= self
                .local
                .entry(key)
                .or_insert_with(Value::unknown)
                .join(&value)?;
        }
        Ok(changed)
    }
    fn reg(&self, r: u32) -> Value {
        self.regs.get(&r).cloned().unwrap_or_else(Value::unknown)
    }
}

pub(super) struct Flow<'i, 'a> {
    index: &'i Index<'a>,
    values: BTreeMap<Binding, Value>,
    readers: BTreeMap<Binding, BTreeSet<u32>>,
    queue: VecDeque<u32>,
    queued: BTreeSet<u32>,
    edges: BTreeSet<Edge>,
    steps: usize,
}

impl<'i, 'a> Flow<'i, 'a> {
    pub fn new(index: &'i Index<'a>) -> Self {
        Self {
            index,
            values: BTreeMap::new(),
            readers: BTreeMap::new(),
            queue: index.methods.keys().copied().collect(),
            queued: index.methods.keys().copied().collect(),
            edges: BTreeSet::new(),
            steps: 0,
        }
    }
    pub fn run(mut self) -> Result<BTreeSet<Edge>> {
        while let Some(method) = self.queue.pop_front() {
            self.queued.remove(&method);
            self.method(method)?;
        }
        Ok(self.edges)
    }
    fn read(&mut self, caller: u32, state: &State, binding: Binding) -> Value {
        if let Some(value) = state.local.get(&binding) {
            return value.clone();
        }
        self.readers
            .entry(binding.clone())
            .or_default()
            .insert(caller);
        self.values
            .get(&binding)
            .cloned()
            .unwrap_or_else(Value::unknown)
    }
    fn publish(&mut self, binding: Binding, value: &Value) -> Result<()> {
        if self
            .values
            .entry(binding.clone())
            .or_default()
            .join(value)?
        {
            if let Some(readers) = self.readers.get(&binding) {
                for &reader in readers {
                    if self.queued.insert(reader) {
                        self.queue.push_back(reader);
                    }
                }
            }
        }
        Ok(())
    }
    fn store(&mut self, state: &mut State, binding: Binding, value: Value) -> Result<()> {
        self.publish(binding.clone(), &value)?;
        state.local.insert(binding, value);
        Ok(())
    }
    fn edge(&mut self, method: u32, ins: &Resolved, kind: EdgeKind, target: Target) {
        self.edges.insert(Edge {
            caller: method,
            pc: ins.ins.pc,
            opcode: ins.ins.opcode.name,
            kind,
            target,
        });
    }
    fn method(&mut self, offset: u32) -> Result<()> {
        let index = self.index;
        let info = &index.methods[&offset];
        let Some(code) = &info.method.code else {
            return Ok(());
        };
        if info.instructions.is_empty() {
            return Ok(());
        }
        let graph = Graph::new(info)?;
        let mut initial = State {
            acc: Value::unknown(),
            regs: BTreeMap::new(),
            env: Value::unknown(),
            local: BTreeMap::new(),
        };
        initial.env = self.read(offset, &initial, Binding::Capture(offset));
        if initial.env.atoms.is_empty() {
            initial.env = Value::one(Atom::Environment(Environment {
                method: offset,
                pc: u32::MAX,
            }));
        }
        for arg in 0..code.arguments {
            let value = if arg == 0 && code.arguments >= 3 {
                Value::one(Atom::Function(offset))
            } else if arg == 2 && code.arguments >= 3 {
                if let Some(owner) = index.owners.get(&offset) {
                    Value::one(if owner.static_ {
                        Atom::Class(owner.class)
                    } else {
                        Atom::Instance(owner.class)
                    })
                } else {
                    self.read(offset, &initial, Binding::Argument(offset, arg))
                }
            } else {
                self.read(offset, &initial, Binding::Argument(offset, arg))
            };
            initial.regs.insert(code.registers + arg, value);
        }
        let mut entries = vec![None; graph.blocks.len()];
        entries[0] = Some(initial);
        let mut queue = VecDeque::from([0]);
        let mut queued = BTreeSet::from([0]);
        while let Some(block) = queue.pop_front() {
            queued.remove(&block);
            let mut state = entries[block]
                .clone()
                .expect("queued block has entry state");
            for ins in &info.instructions[graph.blocks[block].start..graph.blocks[block].end] {
                self.steps += 1;
                if self.steps > 20_000_000 {
                    return Err(Error::invalid(
                        "xref analysis exceeds 20 million instruction visits",
                    ));
                }
                // Exceptions observe registers before the throwing instruction writes its result.
                if ins.ins.opcode.flags & 0x04000000 == 0 {
                    for protected in &code.tries {
                        if ins.ins.pc < protected.start || ins.ins.pc >= protected.end {
                            continue;
                        }
                        for handler in &protected.handlers {
                            let mut exceptional = state.clone();
                            exceptional.acc = Value::unknown();
                            merge(
                                &mut entries,
                                &mut queue,
                                &mut queued,
                                graph.at(handler.pc)?,
                                &exceptional,
                            )?;
                        }
                    }
                }
                self.step(info, ins, &mut state)?;
            }
            for &next in &graph.blocks[block].next {
                merge(&mut entries, &mut queue, &mut queued, next, &state)?;
            }
        }
        Ok(())
    }
    fn capture(&mut self, methods: impl IntoIterator<Item = u32>, env: &Value) -> Result<()> {
        for method in methods {
            self.publish(Binding::Capture(method), env)?;
        }
        Ok(())
    }
    fn lexical(
        &mut self,
        caller: u32,
        state: &State,
        level: i64,
        slot: i64,
    ) -> Result<Vec<Binding>> {
        let level = usize::try_from(level)
            .ok()
            .filter(|level| *level <= 64)
            .ok_or_else(|| Error::invalid("invalid xref lexical depth"))?;
        let slot = u32::try_from(slot).map_err(|_| Error::invalid("invalid xref lexical slot"))?;
        let mut env = state.env.clone();
        for _ in 0..level {
            let mut parents = Value::default();
            for atom in &env.atoms {
                if let Atom::Environment(env) = atom {
                    parents.join(&self.read(caller, state, Binding::Parent(*env)))?;
                }
            }
            env = parents;
        }
        Ok(env
            .atoms
            .into_iter()
            .filter_map(|atom| {
                if let Atom::Environment(env) = atom {
                    Some(Binding::Lexical(env, slot))
                } else {
                    None
                }
            })
            .collect())
    }
    fn step(&mut self, info: &MethodInfo<'_>, ins: &Resolved, state: &mut State) -> Result<()> {
        let index = self.index;
        let caller = info.method.offset;
        let record = info.record;
        let name = base(ins.ins.opcode.name);
        let imms = ins.imms();
        let key = ins.string().map(|off| index.strings[&off].clone());
        let acc = state.acc.clone();
        for reference in &ins.references {
            match reference.kind {
                IdKind::Method => self.edge(
                    caller,
                    ins,
                    EdgeKind::Function,
                    Target::Method(reference.offset),
                ),
                IdKind::Type => {
                    if let Some(record) = index.records.iter().position(|class| {
                        index
                            .file
                            .string(reference.offset)
                            .is_ok_and(|name| class.name == name)
                    }) {
                        self.edge(caller, ins, EdgeKind::Class, Target::Record(record));
                    }
                }
                IdKind::Literal => {
                    let methods = index.literal_methods(reference.offset, &mut BTreeSet::new());
                    self.capture(methods.iter().copied(), &state.env)?;
                    for target in methods {
                        self.edge(caller, ins, EdgeKind::Function, Target::Method(target));
                    }
                }
                IdKind::String | IdKind::Field => {}
            }
        }
        match name {
            "lda" => state.acc = state.reg(ins.reg(0)?),
            "sta" => {
                state.regs.insert(ins.reg(0)?, acc);
            }
            "mov" => {
                state.regs.insert(ins.reg(0)?, state.reg(ins.reg(1)?));
            }
            "lda.str" => {
                state.acc = Value::one(Atom::String(
                    key.ok_or_else(|| Error::invalid("missing string operand"))?,
                ))
            }
            "definefunc" | "definemethod" => {
                let target = ins
                    .method()
                    .ok_or_else(|| Error::invalid("missing method operand"))?;
                self.capture([target], &state.env)?;
                state.acc = Value::one(Atom::Function(target));
            }
            "defineclasswithbuffer" | "callruntime.definesendableclass" => {
                let class = ins
                    .method()
                    .ok_or_else(|| Error::invalid("missing constructor"))?;
                self.edge(caller, ins, EdgeKind::Class, Target::Class(class));
                self.capture([class], &state.env)?;
                self.publish(Binding::Base(class), &state.reg(ins.reg(0)?))?;
                state.acc = Value::one(Atom::Class(class));
            }
            "ldthis" => state.acc = self.this(info, state),
            "ldfunction" => state.acc = Value::one(Atom::Function(caller)),
            "ldglobal" => state.acc = Value::one(Atom::Symbol("globalThis".into())),
            "newlexenv"
            | "newlexenvwithname"
            | "callruntime.newsendableenv"
            | "callruntime.widenewsendableenv" => {
                let env = Environment {
                    method: caller,
                    pc: ins.ins.pc,
                };
                self.publish(Binding::Parent(env), &state.env)?;
                state.env = Value::one(Atom::Environment(env));
            }
            "poplexenv" => {
                let mut result = Value::default();
                for env in &state.env.atoms {
                    if let Atom::Environment(env) = env {
                        result.join(&self.read(caller, state, Binding::Parent(*env)))?;
                    }
                }
                state.env = result;
            }
            "ldlexvar"
            | "stlexvar"
            | "callruntime.ldsendablevar"
            | "callruntime.wideldsendablevar"
            | "callruntime.stsendablevar"
            | "callruntime.widestsendablevar" => {
                let bindings = self.lexical(
                    caller,
                    state,
                    *imms
                        .first()
                        .ok_or_else(|| Error::invalid("missing lexical level"))?,
                    *imms
                        .get(1)
                        .ok_or_else(|| Error::invalid("missing lexical slot"))?,
                )?;
                if name.starts_with("st") || name.contains("stsendable") {
                    let value = if ins.ins.opcode.name.starts_with("deprecated.") {
                        state.reg(ins.reg(0)?)
                    } else {
                        acc
                    };
                    for binding in bindings {
                        self.store(state, binding, value.clone())?;
                    }
                } else {
                    let mut result = Value::default();
                    for binding in bindings {
                        result.join(&self.read(caller, state, binding))?;
                    }
                    state.acc = result;
                }
            }
            "tryldglobalbyname" | "ldglobalvar" => {
                let name = key.ok_or_else(|| Error::invalid("missing global name"))?;
                self.edge(
                    caller,
                    ins,
                    EdgeKind::Read,
                    Target::Named {
                        owner: index.records[record].name.clone(),
                        name: name.clone(),
                    },
                );
                let mut result = self.read(caller, state, Binding::Global(record, name.clone()));
                result.atoms.insert(Atom::Symbol(name));
                state.acc = result;
            }
            "stglobalvar"
            | "trystglobalbyname"
            | "sttoglobalrecord"
            | "stconsttoglobalrecord"
            | "stlettoglobalrecord"
            | "stclasstoglobalrecord" => {
                let name = key.ok_or_else(|| Error::invalid("missing global name"))?;
                self.edge(
                    caller,
                    ins,
                    EdgeKind::Write,
                    Target::Named {
                        owner: index.records[record].name.clone(),
                        name: name.clone(),
                    },
                );
                self.store(state, Binding::Global(record, name), acc)?;
            }
            "stmodulevar"
            | "ldlocalmodulevar"
            | "callruntime.ldsendablelocalmodulevar"
            | "callruntime.wideldsendablelocalmodulevar" => {
                let slot = usize::try_from(
                    *imms
                        .first()
                        .ok_or_else(|| Error::invalid("missing module slot"))?,
                )
                .map_err(|_| Error::invalid("negative module slot"))?;
                if name == "stmodulevar" {
                    self.store(state, Binding::Module(record, slot), acc)?;
                } else {
                    state.acc = self.read(caller, state, Binding::Module(record, slot));
                }
            }
            "ldexternalmodulevar"
            | "callruntime.ldlazymodulevar"
            | "callruntime.wideldlazymodulevar"
            | "callruntime.ldsendableexternalmodulevar"
            | "callruntime.wideldsendableexternalmodulevar"
            | "callruntime.ldlazysendablemodulevar"
            | "callruntime.wideldlazysendablemodulevar" => {
                let slot = usize::try_from(
                    *imms
                        .first()
                        .ok_or_else(|| Error::invalid("missing import slot"))?,
                )
                .map_err(|_| Error::invalid("negative import slot"))?;
                if let Some(import) = index.records[record].module.imports.get(slot) {
                    let symbol = if import.imported.as_deref() == Some("default") {
                        import.local.as_str()
                    } else {
                        import.imported.as_deref().unwrap_or(&import.local)
                    };
                    self.edge(
                        caller,
                        ins,
                        EdgeKind::Import,
                        Target::Named {
                            owner: import.path.clone(),
                            name: symbol.to_string(),
                        },
                    );
                    state.acc = self.import(
                        caller,
                        state,
                        record,
                        &import.path,
                        import.imported.as_deref().unwrap_or("*"),
                        symbol,
                    )?;
                } else {
                    return Err(Error::invalid("xref import slot exceeds module imports"));
                }
            }
            "getmodulenamespace" => {
                let slot = usize::try_from(
                    *imms
                        .first()
                        .ok_or_else(|| Error::invalid("missing namespace slot"))?,
                )
                .map_err(|_| Error::invalid("negative namespace slot"))?;
                let path = index.records[record]
                    .module
                    .requests
                    .get(slot)
                    .ok_or_else(|| Error::invalid("namespace slot exceeds module requests"))?;
                state.acc = self.import(caller, state, record, path, "*", "*")?;
            }
            "ldobjbyname" | "ldthisbyname" | "ldsuperbyname" => {
                let key = key.ok_or_else(|| Error::invalid("missing property name"))?;
                let object = if name == "ldthisbyname" {
                    self.this(info, state)
                } else if name == "ldsuperbyname" {
                    self.super_(caller, state)?
                } else if ins.ins.opcode.name.starts_with("deprecated.") {
                    state.reg(ins.reg(0)?)
                } else {
                    acc
                };
                state.acc = self.property(caller, ins, state, &object, &key, None)?;
            }
            "ldobjbyvalue" | "ldthisbyvalue" | "ldsuperbyvalue" => {
                let (object, keys) = if name == "ldthisbyvalue" {
                    (self.this(info, state), acc)
                } else if ins.ins.opcode.name.starts_with("deprecated.") {
                    (state.reg(ins.reg(0)?), state.reg(ins.reg(1)?))
                } else if name == "ldsuperbyvalue" {
                    (self.super_(caller, state)?, acc)
                } else {
                    (state.reg(ins.reg(0)?), acc)
                };
                let mut result = Value::default();
                for key in &keys.atoms {
                    if let Atom::String(key) = key {
                        result.join(&self.property(caller, ins, state, &object, key, None)?)?;
                    }
                }
                if result.atoms.is_empty() {
                    result = Value::unknown();
                }
                state.acc = result;
            }
            "stobjbyname"
            | "stownbyname"
            | "stownbynamewithnameset"
            | "definefieldbyname"
            | "definepropertybyname"
            | "stthisbyname"
            | "stsuperbyname" => {
                let key = key.ok_or_else(|| Error::invalid("missing property name"))?;
                let object = if name == "stthisbyname" {
                    self.this(info, state)
                } else if name == "stsuperbyname" {
                    self.super_(caller, state)?
                } else {
                    state.reg(ins.reg(0)?)
                };
                self.property(caller, ins, state, &object, &key, Some(&acc))?;
            }
            "stobjbyvalue"
            | "stownbyvalue"
            | "stownbyvaluewithnameset"
            | "stthisbyvalue"
            | "stsuperbyvalue"
            | "callruntime.definefieldbyvalue" => {
                let object = if name == "stthisbyvalue" {
                    self.this(info, state)
                } else if name == "stsuperbyvalue" {
                    self.super_(caller, state)?
                } else {
                    state.reg(ins.reg(0)?)
                };
                let keys = state.reg(ins.reg(if name == "stthisbyvalue" { 0 } else { 1 })?);
                for key in &keys.atoms {
                    if let Atom::String(key) = key {
                        self.property(caller, ins, state, &object, key, Some(&acc))?;
                    }
                }
            }
            "ldobjbyindex" | "stobjbyindex" | "stownbyindex" | "callruntime.definefieldbyindex" => {
                let key = imms
                    .last()
                    .ok_or_else(|| Error::invalid("missing property index"))?
                    .to_string();
                if name == "ldobjbyindex" {
                    state.acc = self.property(caller, ins, state, &acc, &key, None)?;
                } else {
                    self.property(
                        caller,
                        ins,
                        state,
                        &state.reg(ins.reg(0)?),
                        &key,
                        Some(&acc),
                    )?;
                }
            }
            "createemptyobject"
            | "createemptyarray"
            | "createobjectwithbuffer"
            | "createarraywithbuffer" => {
                let object = Atom::Object {
                    method: caller,
                    pc: ins.ins.pc,
                };
                if let Some(buffer) = ins.literal() {
                    let mut i = 0;
                    let values = &index.literals[&buffer];
                    while i < values.len() {
                        let (key, value) = if name == "createobjectwithbuffer" {
                            if let Some((_, value)) = values.get(i + 1) {
                                let key = match &values[i].1 {
                                    Literal::String(key) | Literal::Value(key) => key.clone(),
                                    _ => {
                                        i += 2;
                                        continue;
                                    }
                                };
                                (key, value)
                            } else {
                                break;
                            }
                        } else {
                            (i.to_string(), &values[i].1)
                        };
                        let value = match value {
                            Literal::Method { offset, .. } => Value::one(Atom::Function(*offset)),
                            Literal::String(text) => Value::one(Atom::String(text.clone())),
                            Literal::Value(_) | Literal::Array(_) | Literal::Affiliate(_) => {
                                Value::unknown()
                            }
                        };
                        self.store(state, Binding::Property(object.clone(), key), value)?;
                        i += if name == "createobjectwithbuffer" {
                            2
                        } else {
                            1
                        };
                        if values
                            .get(i)
                            .is_some_and(|(_, v)| matches!(v, Literal::Affiliate(_)))
                        {
                            i += 1;
                        }
                    }
                }
                state.acc = Value::one(object);
            }
            "newobjrange" | "newobjapply" => {
                let function = state.reg(ins.reg(0)?);
                let args = self.arguments(ins, state, true)?;
                self.call(
                    (caller, ins, state),
                    &function,
                    &Value::unknown(),
                    &args,
                    EdgeKind::Construct,
                )?;
                let mut result = Value::default();
                for target in function.atoms {
                    match target {
                        Atom::Class(off) => {
                            result.atoms.insert(Atom::Instance(off));
                        }
                        Atom::Symbol(name) => {
                            result.atoms.insert(Atom::Symbol(format!("new {name}")));
                        }
                        Atom::Function(_)
                        | Atom::Instance(_)
                        | Atom::Prototype(_)
                        | Atom::Object { .. }
                        | Atom::String(_)
                        | Atom::Environment(_) => {
                            result.unknown = true;
                        }
                    }
                }
                state.acc = result;
            }
            "supercallthisrange"
            | "supercallarrowrange"
            | "supercallspread"
            | "callruntime.supercallforwardallargs" => {
                let function = self.super_(caller, state)?;
                let args = self.arguments(ins, state, false)?;
                state.acc = self.call(
                    (caller, ins, state),
                    &function,
                    &self.this(info, state),
                    &args,
                    EdgeKind::Construct,
                )?;
            }
            "callarg0"
            | "callarg1"
            | "callargs2"
            | "callargs3"
            | "callrange"
            | "callthis0"
            | "callthis1"
            | "callthis2"
            | "callthis3"
            | "callthisrange"
            | "callthis0withname"
            | "callthis1withname"
            | "callthis2withname"
            | "callthis3withname"
            | "callthisrangewithname"
            | "apply"
            | "callspread"
            | "callruntime.callinit" => {
                let deprecated = ins.ins.opcode.name.starts_with("deprecated.");
                let function = if deprecated {
                    state.reg(ins.reg(0)?)
                } else {
                    acc
                };
                let receiver =
                    if name.starts_with("callthis") || name == "apply" || name == "callspread" {
                        state.reg(ins.reg(usize::from(deprecated))?)
                    } else if name == "callruntime.callinit" {
                        self.this(info, state)
                    } else {
                        Value::unknown()
                    };
                let args = self.arguments(ins, state, false)?;
                // The withname operand is a call-site name; the accumulator still holds the callee.
                let function = if function.atoms.is_empty() {
                    if let Some(key) = key {
                        self.property(caller, ins, state, &receiver, &key, None)?
                    } else {
                        function
                    }
                } else {
                    function
                };
                state.acc = self.call(
                    (caller, ins, state),
                    &function,
                    &receiver,
                    &args,
                    EdgeKind::Call,
                )?;
            }
            "return" | "asyncfunctionresolve" => self.publish(Binding::Return(caller), &acc)?,
            "nop" | "debugger" | "returnundefined" | "setgeneratorstate" | "ldpatchvar"
            | "stpatchvar" => {}
            _ if ins.ins.opcode.flags & JUMP != 0 || name.starts_with("throw") => {}
            _ => {
                if ins.ins.opcode.flags & 0x20000000 != 0 || matches!(name, "ldai" | "fldai") {
                    state.acc = Value::unknown();
                }
            }
        }
        Ok(())
    }
    fn this(&self, info: &MethodInfo<'_>, state: &State) -> Value {
        let Some(code) = &info.method.code else {
            return Value::unknown();
        };
        if code.arguments >= 3 {
            state.reg(code.registers + 2)
        } else {
            Value::unknown()
        }
    }
    fn super_(&mut self, caller: u32, state: &State) -> Result<Value> {
        if let Some(owner) = self.index.owners.get(&caller) {
            Ok(self.read(caller, state, Binding::Base(owner.class)))
        } else {
            Ok(Value::one(Atom::Symbol("super".into())))
        }
    }
    fn arguments(&self, ins: &Resolved, state: &State, constructor: bool) -> Result<Vec<Value>> {
        let name = base(ins.ins.opcode.name);
        let regs = ins
            .ins
            .operands
            .iter()
            .filter_map(|op| {
                if let crate::Operand::Register(r) = op {
                    Some(*r)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        if name.contains("range") {
            let count = u32::try_from(
                *ins.imms()
                    .last()
                    .ok_or_else(|| Error::invalid("missing call argument count"))?,
            )
            .ok()
            .filter(|n| *n <= 65536)
            .ok_or_else(|| Error::invalid("invalid call argument count"))?;
            let first = *regs
                .first()
                .ok_or_else(|| Error::invalid("missing call argument range"))?;
            let skip = u32::from(constructor || name.starts_with("callthis"));
            let count = if constructor {
                count.saturating_sub(1)
            } else {
                count
            };
            (0..count)
                .map(|i| {
                    first
                        .checked_add(skip)
                        .and_then(|r| r.checked_add(i))
                        .map(|r| state.reg(r))
                        .ok_or_else(|| Error::invalid("xref argument range overflow"))
                })
                .collect()
        } else if matches!(
            name,
            "apply" | "callspread" | "newobjapply" | "supercallspread"
        ) {
            Ok(Vec::new())
        } else {
            let skip = usize::from(ins.ins.opcode.name.starts_with("deprecated."))
                + usize::from(name.starts_with("callthis"));
            Ok(regs.into_iter().skip(skip).map(|r| state.reg(r)).collect())
        }
    }
    fn call(
        &mut self,
        context: (u32, &Resolved, &State),
        function: &Value,
        receiver: &Value,
        args: &[Value],
        kind: EdgeKind,
    ) -> Result<Value> {
        let (caller, ins, state) = context;
        let mut result = Value::default();
        for atom in &function.atoms {
            match atom {
                Atom::Function(off) | Atom::Class(off) => {
                    self.edge(
                        caller,
                        ins,
                        kind,
                        if matches!(atom, Atom::Class(_)) {
                            Target::Class(*off)
                        } else {
                            Target::Method(*off)
                        },
                    );
                    if matches!(atom, Atom::Class(_)) {
                        self.edge(caller, ins, kind, Target::Method(*off));
                    }
                    self.publish(Binding::Argument(*off, 2), receiver)?;
                    for (i, arg) in args.iter().enumerate() {
                        self.publish(
                            Binding::Argument(
                                *off,
                                u32::try_from(i)
                                    .map_err(|_| Error::invalid("too many xref arguments"))?
                                    + 3,
                            ),
                            arg,
                        )?;
                    }
                    result.join(&self.read(caller, state, Binding::Return(*off)))?;
                }
                Atom::Symbol(symbol) => {
                    self.edge(caller, ins, kind, symbolic(symbol));
                    result.unknown = true;
                }
                Atom::Instance(_)
                | Atom::Prototype(_)
                | Atom::Object { .. }
                | Atom::String(_)
                | Atom::Environment(_) => {
                    result.unknown = true;
                }
            }
        }
        if function.atoms.is_empty() {
            self.edge(
                caller,
                ins,
                kind,
                Target::Named {
                    owner: String::new(),
                    name: "<dynamic>".into(),
                },
            );
            result.unknown = true;
        }
        Ok(result)
    }
    fn property(
        &mut self,
        caller: u32,
        ins: &Resolved,
        state: &mut State,
        object: &Value,
        key: &str,
        store: Option<&Value>,
    ) -> Result<Value> {
        let index = self.index;
        let mut result = Value::default();
        for atom in &object.atoms {
            let owner = match atom {
                Atom::Class(off) | Atom::Instance(off) | Atom::Prototype(off) => {
                    index.classes[off].name.clone()
                }
                Atom::Symbol(symbol) => symbol.strip_prefix("new ").unwrap_or(symbol).to_string(),
                Atom::Object { .. } => String::new(),
                Atom::Function(off) => index.method_name(*off),
                Atom::String(_) | Atom::Environment(_) => String::new(),
            };
            self.edge(
                caller,
                ins,
                if store.is_some() {
                    EdgeKind::Write
                } else {
                    EdgeKind::Read
                },
                Target::Named {
                    owner: owner.clone(),
                    name: key.to_string(),
                },
            );
            let binding = Binding::Property(atom.clone(), key.to_string());
            if let Some(value) = store {
                self.store(state, binding, value.clone())?;
            } else {
                result.join(&self.read(caller, state, binding))?;
            }
            match atom {
                Atom::Class(off) | Atom::Instance(off) | Atom::Prototype(off) => {
                    if key == "prototype" && matches!(atom, Atom::Class(_)) {
                        result.atoms.insert(Atom::Prototype(*off));
                        continue;
                    }
                    let static_ = matches!(atom, Atom::Class(_));
                    let mut candidates =
                        self.members(caller, state, *off, static_, key, &mut BTreeSet::new())?;
                    for (method, tag) in candidates.drain(..) {
                        if tag == 0x1a && store.is_none() {
                            result.join(&self.call(
                                (caller, ins, state),
                                &Value::one(Atom::Function(method)),
                                object,
                                &[],
                                EdgeKind::Getter,
                            )?)?;
                        } else if tag == 0x1b {
                            if let Some(value) = store {
                                self.call(
                                    (caller, ins, state),
                                    &Value::one(Atom::Function(method)),
                                    object,
                                    std::slice::from_ref(value),
                                    EdgeKind::Setter,
                                )?;
                            }
                        } else if tag != 0x1a && store.is_none() {
                            result.atoms.insert(Atom::Function(method));
                        }
                    }
                    if key == "constructor" && !static_ {
                        result.atoms.insert(Atom::Class(*off));
                    }
                }
                Atom::Symbol(symbol) => {
                    if symbol.starts_with("namespace:") {
                        if let Some(record) = symbol
                            .strip_prefix("namespace:")
                            .and_then(|n| n.parse::<usize>().ok())
                        {
                            result.join(&self.export(
                                caller,
                                state,
                                record,
                                key,
                                &mut BTreeSet::new(),
                            )?)?;
                        }
                    } else if symbol.len() + key.len() <= 1024 {
                        result.atoms.insert(Atom::Symbol(format!("{owner}.{key}")));
                    } else {
                        return Err(Error::invalid(
                            "xref symbolic property path exceeds 1024 bytes",
                        ));
                    }
                }
                Atom::Function(off) if key == "call" || key == "apply" => {
                    result.atoms.insert(Atom::Function(*off));
                }
                Atom::Function(_)
                | Atom::Object { .. }
                | Atom::String(_)
                | Atom::Environment(_) => {}
            }
        }
        if object.atoms.is_empty() {
            self.edge(
                caller,
                ins,
                if store.is_some() {
                    EdgeKind::Write
                } else {
                    EdgeKind::Read
                },
                Target::Named {
                    owner: String::new(),
                    name: key.to_string(),
                },
            );
            result
                .atoms
                .insert(Atom::Symbol(format!("<dynamic>.{key}")));
        }
        Ok(result)
    }
    fn members(
        &mut self,
        caller: u32,
        state: &State,
        class: u32,
        static_: bool,
        key: &str,
        visited: &mut BTreeSet<u32>,
    ) -> Result<Vec<(u32, u8)>> {
        if !visited.insert(class) {
            return Ok(Vec::new());
        }
        if let Some(members) = self.index.classes[&class]
            .members
            .get(&(static_, key.to_string()))
        {
            return Ok(members.clone());
        }
        let parents = self.read(caller, state, Binding::Base(class));
        let mut result = Vec::new();
        for parent in parents.atoms {
            if let Atom::Class(parent) = parent {
                result.extend(self.members(caller, state, parent, static_, key, visited)?);
            }
        }
        Ok(result)
    }
    fn import(
        &mut self,
        caller: u32,
        state: &State,
        record: usize,
        path: &str,
        imported: &str,
        local: &str,
    ) -> Result<Value> {
        if let Some(target) = self.record(record, path) {
            if imported == "*" {
                Ok(Value::one(Atom::Symbol(format!("namespace:{target}"))))
            } else {
                self.export(caller, state, target, imported, &mut BTreeSet::new())
            }
        } else {
            Ok(Value::one(Atom::Symbol(format!("{path}::{local}"))))
        }
    }
    fn export(
        &mut self,
        caller: u32,
        state: &State,
        record: usize,
        name: &str,
        visited: &mut BTreeSet<(usize, String)>,
    ) -> Result<Value> {
        if !visited.insert((record, name.to_string())) {
            return Ok(Value::default());
        }
        let index = self.index;
        let mut result = Value::default();
        for (slot, export) in index.records[record].module.exports.iter().enumerate() {
            if export.exported.as_deref() == Some(name) && export.path.is_none() {
                result.join(&self.read(caller, state, Binding::Module(record, slot)))?;
            } else if let Some(path) = &export.path {
                if export.exported.as_deref() == Some(name) || export.exported.is_none() {
                    if let Some(target) = self.record(record, path) {
                        result.join(&self.export(
                            caller,
                            state,
                            target,
                            export.local.as_deref().unwrap_or(name),
                            visited,
                        )?)?;
                    }
                }
            }
        }
        Ok(result)
    }
    pub(super) fn record(&self, source: usize, path: &str) -> Option<usize> {
        // Normalized module URLs carry flags before the path and version fields after it.
        // Preserve the actual path instead of cutting at the first ampersand in the prefix.
        let clean = |text: &str| {
            let text = text
                .strip_prefix("@normalized:")
                .and_then(|text| text.split_once("&&&").map(|(_, path)| path))
                .unwrap_or(text);
            let text = text
                .strip_prefix('L')
                .unwrap_or(text)
                .trim_end_matches(';')
                .trim_matches('&');
            let text = text.split('&').next().unwrap_or(text);
            text.trim_end_matches(".ets")
                .trim_end_matches(".js")
                .to_string()
        };
        let path = clean(path);
        let source = clean(&self.index.records[source].name);
        let resolved = if path.starts_with('.') {
            let mut parts = source
                .rsplit_once('/')
                .map_or("", |(dir, _)| dir)
                .split('/')
                .filter(|part| !part.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>();
            for part in path.split('/') {
                match part {
                    "." | "" => {}
                    ".." => {
                        parts.pop();
                    }
                    part => parts.push(part.to_string()),
                }
            }
            parts.join("/")
        } else {
            clean(&path)
        };
        let mut matches = self
            .index
            .records
            .iter()
            .enumerate()
            .filter_map(|(index, record)| {
                let record = clean(&record.name);
                (record == resolved || record == clean(&path)).then_some(index)
            });
        let first = matches.next()?;
        // An ambiguous import remains symbolic rather than guessing one definition.
        if matches.next().is_some() {
            None
        } else {
            Some(first)
        }
    }
}

fn symbolic(symbol: &str) -> Target {
    let (owner, name) = symbol
        .rsplit_once('.')
        .or_else(|| symbol.rsplit_once("::"))
        .unwrap_or(("", symbol));
    Target::Named {
        owner: if owner == "<dynamic>" {
            String::new()
        } else {
            owner.to_string()
        },
        name: name.to_string(),
    }
}

struct Block {
    start: usize,
    end: usize,
    next: Vec<usize>,
}
struct Graph {
    blocks: Vec<Block>,
    pcs: BTreeMap<u32, usize>,
}
impl Graph {
    fn new(info: &MethodInfo<'_>) -> Result<Self> {
        let mut leaders = BTreeSet::from([0]);
        let end = info
            .instructions
            .last()
            .expect("nonempty method")
            .ins
            .next_pc;
        for ins in &info.instructions {
            if let Some(target) = ins.ins.jump_target()? {
                leaders.insert(target);
                leaders.insert(ins.ins.next_pc);
            }
            if ins.ins.opcode.flags & 0x80 != 0 || ins.ins.opcode.name == "throw" {
                leaders.insert(ins.ins.next_pc);
            }
        }
        if let Some(code) = &info.method.code {
            for protected in &code.tries {
                leaders.insert(protected.start);
                leaders.insert(protected.end);
                for handler in &protected.handlers {
                    leaders.insert(handler.pc);
                }
            }
        }
        leaders.remove(&end);
        let positions: BTreeMap<_, _> = info
            .instructions
            .iter()
            .enumerate()
            .map(|(i, ins)| (ins.ins.pc, i))
            .collect();
        let mut pcs = BTreeMap::new();
        let mut blocks = Vec::new();
        for pc in leaders {
            let start = *positions.get(&pc).ok_or_else(|| {
                Error::invalid(format!(
                    "xref control flow target {pc:#x} is not an instruction boundary"
                ))
            })?;
            pcs.insert(pc, blocks.len());
            blocks.push(Block {
                start,
                end: info.instructions.len(),
                next: Vec::new(),
            });
        }
        for i in 0..blocks.len() {
            if let Some(next) = blocks.get(i + 1) {
                blocks[i].end = next.start;
            }
            let ins = &info.instructions[blocks[i].end - 1].ins;
            if let Some(target) = ins.jump_target()? {
                if target != end {
                    blocks[i].next.push(
                        *pcs.get(&target)
                            .ok_or_else(|| Error::invalid("xref jump target is outside method"))?,
                    );
                }
            }
            if (ins.opcode.flags & JUMP == 0 || ins.opcode.flags & CONDITIONAL != 0)
                && ins.opcode.flags & 0x80 == 0
                && ins.opcode.name != "throw"
                && i + 1 < blocks.len()
            {
                blocks[i].next.push(i + 1);
            }
        }
        Ok(Self { blocks, pcs })
    }
    fn at(&self, pc: u32) -> Result<usize> {
        self.pcs
            .get(&pc)
            .copied()
            .ok_or_else(|| Error::invalid("xref handler target is outside method"))
    }
}

fn merge(
    entries: &mut [Option<State>],
    queue: &mut VecDeque<usize>,
    queued: &mut BTreeSet<usize>,
    block: usize,
    state: &State,
) -> Result<()> {
    let changed = if let Some(entry) = &mut entries[block] {
        entry.join(state)?
    } else {
        entries[block] = Some(state.clone());
        true
    };
    if changed && queued.insert(block) {
        queue.push_back(block);
    }
    Ok(())
}
