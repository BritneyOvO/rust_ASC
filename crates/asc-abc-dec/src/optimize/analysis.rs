use std::collections::{BTreeMap, BTreeSet};

use oxc::ast::ast::*;
use oxc::ast_visit::{walk, Visit};
use oxc::syntax::{operator::AssignmentOperator, scope::ScopeFlags};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum UsageMode {
    #[default]
    Full,
    Eager,
    Writes,
}

// Names belong to the AST arena, so replacing nodes does not invalidate these
// slices. Keeping them borrowed avoids allocating a String for every access.
#[derive(Default)]
pub(super) struct Usage<'a> {
    pub reads: BTreeMap<&'a str, usize>,
    pub writes: BTreeSet<&'a str>,
    pub assigned: BTreeSet<&'a str>,
    pub assignments: BTreeMap<&'a str, usize>,
    pub captured: BTreeSet<&'a str>,
    depth: usize,
    shadowed: BTreeSet<&'a str>,
    mode: UsageMode,
}

impl<'a> Usage<'a> {
    pub fn statements(statements: &[Statement<'a>]) -> Self {
        let mut usage = Self::default();
        for statement in statements {
            usage.visit_statement(statement);
        }
        usage
    }

    pub fn statement(statement: &Statement<'a>) -> Self {
        let mut usage = Self::default();
        usage.visit_statement(statement);
        usage
    }

    pub fn expression(expression: &Expression<'a>) -> Self {
        let mut usage = Self::default();
        usage.visit_expression(expression);
        usage
    }

    pub fn eager_expression(expression: &Expression<'a>) -> Self {
        let mut usage = Self {
            mode: UsageMode::Eager,
            ..Self::default()
        };
        usage.visit_expression(expression);
        usage
    }

    pub fn count(&self, name: &str) -> usize {
        self.reads.get(name).copied().unwrap_or(0)
    }

    pub fn writes(statements: &[Statement<'a>]) -> BTreeSet<&'a str> {
        let mut usage = Self {
            mode: UsageMode::Writes,
            ..Self::default()
        };
        for statement in statements {
            usage.visit_statement(statement);
        }
        usage.writes
    }

    pub fn statement_writes(statement: &Statement<'a>) -> BTreeSet<&'a str> {
        Self::writes(std::slice::from_ref(statement))
    }
}

/// Statement indices remain stable until the containing pass removes empty
/// statements. Invalidate an entry whenever that pass rewrites its AST.
pub(super) struct StatementUses<'a>(Vec<Option<Usage<'a>>>);

impl<'a> StatementUses<'a> {
    pub fn new(len: usize) -> Self {
        Self(std::iter::repeat_with(|| None).take(len).collect())
    }

    pub fn get(&mut self, index: usize, statement: &Statement<'a>) -> &Usage<'a> {
        self.0[index].get_or_insert_with(|| Usage::statement(statement))
    }

    pub fn invalidate(&mut self, index: usize) {
        self.0[index] = None;
    }
}

impl<'a> Visit<'a> for Usage<'a> {
    fn visit_identifier_reference(&mut self, identifier: &IdentifierReference<'a>) {
        if self.mode == UsageMode::Writes || self.shadowed.contains(identifier.name.as_str()) {
            return;
        }
        *self.reads.entry(identifier.name.as_str()).or_default() += 1;
        if self.depth != 0 {
            self.captured.insert(identifier.name.as_str());
        }
    }

    fn visit_assignment_expression(&mut self, expression: &AssignmentExpression<'a>) {
        if let AssignmentTarget::AssignmentTargetIdentifier(identifier) = &expression.left {
            if !self.shadowed.contains(identifier.name.as_str()) {
                self.writes.insert(identifier.name.as_str());
                if self.mode != UsageMode::Writes {
                    self.assigned.insert(identifier.name.as_str());
                    *self
                        .assignments
                        .entry(identifier.name.as_str())
                        .or_default() += 1;
                    if self.depth != 0 {
                        self.captured.insert(identifier.name.as_str());
                    }
                }
            }
            if expression.operator != AssignmentOperator::Assign {
                self.visit_identifier_reference(identifier);
            }
        } else {
            self.visit_assignment_target(&expression.left);
        }
        self.visit_expression(&expression.right);
    }

    fn visit_update_expression(&mut self, expression: &UpdateExpression<'a>) {
        if let SimpleAssignmentTarget::AssignmentTargetIdentifier(identifier) = &expression.argument
        {
            if !self.shadowed.contains(identifier.name.as_str()) {
                self.writes.insert(identifier.name.as_str());
                if self.mode != UsageMode::Writes {
                    self.assigned.insert(identifier.name.as_str());
                    *self
                        .assignments
                        .entry(identifier.name.as_str())
                        .or_default() += 1;
                    if self.depth != 0 {
                        self.captured.insert(identifier.name.as_str());
                    }
                }
            }
        }
        walk::walk_update_expression(self, expression);
    }

    fn visit_binding_identifier(&mut self, identifier: &BindingIdentifier<'a>) {
        if !self.shadowed.contains(identifier.name.as_str()) {
            self.writes.insert(identifier.name.as_str());
        }
    }

    fn visit_function(&mut self, function: &Function<'a>, flags: ScopeFlags) {
        if self.mode == UsageMode::Eager {
            return;
        }
        let mut locals = Locals::<&str>::default();
        if let Some(name) = &function.id {
            locals.0.insert(name.name.as_str());
        }
        locals.visit_formal_parameters(&function.params);
        if let Some(body) = &function.body {
            locals.visit_function_body(body);
        }
        let shadowed = self.shadowed.clone();
        self.shadowed.extend(locals.0);
        self.depth += 1;
        walk::walk_function(self, function, flags);
        self.depth -= 1;
        self.shadowed = shadowed;
    }

    fn visit_arrow_function_expression(&mut self, function: &ArrowFunctionExpression<'a>) {
        if self.mode == UsageMode::Eager {
            return;
        }
        let mut locals = Locals::<&str>::default();
        locals.visit_formal_parameters(&function.params);
        locals.visit_function_body(&function.body);
        let shadowed = self.shadowed.clone();
        self.shadowed.extend(locals.0);
        self.depth += 1;
        walk::walk_arrow_function_expression(self, function);
        self.depth -= 1;
        self.shadowed = shadowed;
    }

    fn visit_class(&mut self, class: &Class<'a>) {
        let shadowed = self.shadowed.clone();
        if let Some(name) = &class.id {
            self.shadowed.insert(name.name.as_str());
        }
        walk::walk_class(self, class);
        self.shadowed = shadowed;
    }
}

#[derive(Default)]
pub(super) struct Locals<N = String>(pub BTreeSet<N>);
impl<'a, N: From<&'a str> + Ord> Visit<'a> for Locals<N> {
    fn visit_binding_identifier(&mut self, identifier: &BindingIdentifier<'a>) {
        self.0.insert(identifier.name.as_str().into());
    }
    fn visit_function(&mut self, function: &Function<'a>, flags: ScopeFlags) {
        let _ = flags;
        if function.r#type == FunctionType::FunctionDeclaration {
            if let Some(name) = &function.id {
                self.0.insert(name.name.as_str().into());
            }
        }
    }
    fn visit_class(&mut self, class: &Class<'a>) {
        if class.r#type == ClassType::ClassDeclaration {
            if let Some(name) = &class.id {
                self.0.insert(name.name.as_str().into());
            }
        }
    }
    fn visit_arrow_function_expression(&mut self, function: &ArrowFunctionExpression<'a>) {
        let _ = function;
    }
}

pub(super) fn temporary(name: &str) -> bool {
    matches!(
        name,
        "acc" | "__this" | "__fnObject" | "__newTarget" | "__resume"
    ) || name
        .strip_prefix('v')
        .is_some_and(|tail| !tail.is_empty() && tail.bytes().all(|byte| byte.is_ascii_digit()))
}

pub(super) fn pure(expression: &Expression<'_>) -> bool {
    match expression {
        Expression::BooleanLiteral(_)
        | Expression::NullLiteral(_)
        | Expression::NumericLiteral(_)
        | Expression::BigIntLiteral(_)
        | Expression::StringLiteral(_)
        | Expression::ThisExpression(_)
        | Expression::MetaProperty(_)
        | Expression::FunctionExpression(_)
        | Expression::ArrowFunctionExpression(_) => true,
        Expression::Identifier(identifier) => {
            temporary(&identifier.name)
                || identifier.name == "undefined"
                || matches!(identifier.name.as_str(), "__env" | "__parentEnv" | "__home" | "__hole")
                || identifier.name.starts_with("__scope_") || identifier.name.starts_with("local_")
                || identifier.name.strip_prefix('p').is_some_and(|tail| !tail.is_empty() && tail.bytes().all(|byte| byte.is_ascii_digit()))
        }
        Expression::ParenthesizedExpression(expression) => pure(&expression.expression),
        Expression::UnaryExpression(expression) => {
            matches!(expression.operator.as_str(), "!" | "void" | "typeof")
                && pure(&expression.argument)
        }
        Expression::ArrayExpression(array) => array.elements.iter().all(|element| element.as_expression().is_some_and(pure)),
        Expression::ObjectExpression(object) => object.properties.iter().all(|property| matches!(property,
            ObjectPropertyKind::ObjectProperty(property) if !property.computed && pure(&property.value))),
        _ => false,
    }
}

/// Moving a value into a later expression must preserve the evaluation order
/// of getters, coercions, calls, and conditional operands.
pub(super) fn first_effect_is_use(statement: &Statement<'_>, name: &str) -> bool {
    let mut prefix = Prefix {
        name,
        found: false,
        effect: false,
    };
    prefix.visit_statement(statement);
    prefix.found && !prefix.effect
}

struct Prefix<'n> {
    name: &'n str,
    found: bool,
    effect: bool,
}

impl<'a> Visit<'a> for Prefix<'_> {
    fn visit_if_statement(&mut self, statement: &IfStatement<'a>) {
        self.visit_expression(&statement.test);
        if !self.found {
            self.effect = true;
        }
        self.visit_statement(&statement.consequent);
        if let Some(alternate) = &statement.alternate {
            self.visit_statement(alternate);
        }
    }

    fn visit_while_statement(&mut self, statement: &WhileStatement<'a>) {
        self.visit_expression(&statement.test);
        if !self.found {
            self.effect = true;
        }
        self.visit_statement(&statement.body);
    }

    fn visit_try_statement(&mut self, statement: &TryStatement<'a>) {
        // Moving a throwing read inside a try changes which handler sees it.
        if !self.found {
            self.effect = true;
        }
        walk::walk_try_statement(self, statement);
    }

    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        if let Some(member) = call.callee.get_member_expr() {
            if member.static_property_name() == Some("defineProperty")
                && matches!(member.object(), Expression::Identifier(identifier) if identifier.name == "Object")
            {
                // Own-property stores also use a generated intrinsic wrapper.
                for argument in &call.arguments {
                    self.visit_argument(argument);
                }
                return;
            }
            if member.static_property_name() == Some("call") {
                // CallThis instructions use the function value directly. The
                // .call member is generated scaffolding, not a bytecode read.
                self.visit_expression(member.object());
                for argument in &call.arguments {
                    self.visit_argument(argument);
                }
                return;
            }
        }
        walk::walk_call_expression(self, call);
    }
    fn visit_expression(&mut self, expression: &Expression<'a>) {
        if self.found {
            return;
        }
        if let Expression::Identifier(identifier) = expression {
            if identifier.name == self.name {
                self.found = true;
            }
            return;
        }
        match expression {
            Expression::FunctionExpression(_) | Expression::ArrowFunctionExpression(_) => {
                return;
            }
            Expression::LogicalExpression(expression) => {
                self.visit_expression(&expression.left);
                if !self.found {
                    self.effect = true;
                }
                self.visit_expression(&expression.right);
                return;
            }
            Expression::ConditionalExpression(expression) => {
                self.visit_expression(&expression.test);
                if !self.found {
                    self.effect = true;
                }
                self.visit_expression(&expression.consequent);
                self.visit_expression(&expression.alternate);
                return;
            }
            _ => {}
        }
        walk::walk_expression(self, expression);
        if !self.found && !pure(expression) {
            self.effect = true;
        }
    }

    fn visit_assignment_target(&mut self, target: &AssignmentTarget<'a>) {
        if !matches!(target, AssignmentTarget::AssignmentTargetIdentifier(_)) {
            walk::walk_assignment_target(self, target);
        }
    }
}
