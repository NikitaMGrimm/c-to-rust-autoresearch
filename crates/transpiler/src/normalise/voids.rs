//! `voids`, R43: C writes `void *` where it does not say the type of the
//! memory, and Rust always says it. The slots that hand a `void` pointer on
//! to each other form a group, and the one type of a group is the type of
//! every pointer stored into it and of every pointer it is converted to.
//! Where a group has exactly one, its slots get that type and the
//! conversions go, so that no later stage sees the `void` pointer. A group
//! with no type or with two is refused.

use crate::Unsupported;
use crate::c::{BinOp, Expr, ExprKind, FuncId, Program, Stmt, Type};
use crate::facts::{Slot, place_slot};
use std::collections::HashMap;

type R<T> = Result<T, Unsupported>;

pub fn voids(program: &mut Program) -> R<()> {
    let mut slots = Vec::new();
    for (id, var) in program.vars.iter().enumerate() {
        slots.push((Slot::Var(id), &var.ty, var.at));
    }
    for (id, record) in program.records.iter().enumerate() {
        for (field, declared) in record.fields.iter().enumerate() {
            slots.push((Slot::Field(id, field), &declared.ty, 0));
        }
    }
    for (id, function) in program.functions.iter().enumerate() {
        slots.push((Slot::Return(id), &function.ret, 0));
    }
    let mut voids = Voids::default();
    for (slot, ty, at) in slots {
        nested(ty, at)?;
        if is_void(ty) {
            voids.nodes.insert(slot, voids.parent.len());
            voids.parent.push(voids.parent.len());
            voids.at.push(at);
        }
    }
    if voids.nodes.is_empty() {
        return Ok(());
    }
    voids.functions = program
        .functions
        .iter()
        .enumerate()
        .map(|(id, function)| (function.name.clone(), id))
        .collect();
    voids.params = program
        .functions
        .iter()
        .map(|function| function.params.iter().map(|&var| Slot::Var(var)).collect())
        .collect();
    voids.names = program
        .records
        .iter()
        .map(|record| record.name.clone())
        .collect();
    voids.walk(program)?;
    voids.decide()?;
    voids.rewriting = true;
    voids.walk(program)?;
    for (id, var) in program.vars.iter_mut().enumerate() {
        if let Some(ty) = voids.slot_type(Slot::Var(id)) {
            var.ty = ty;
        }
    }
    for (id, record) in program.records.iter_mut().enumerate() {
        for (field, declared) in record.fields.iter_mut().enumerate() {
            if let Some(ty) = voids.slot_type(Slot::Field(id, field)) {
                declared.ty = ty;
            }
        }
    }
    for (id, function) in program.functions.iter_mut().enumerate() {
        if let Some(ty) = voids.slot_type(Slot::Return(id)) {
            function.ret = ty;
        }
    }
    Ok(())
}

fn is_void(ty: &Type) -> bool {
    matches!(ty, Type::Pointer(to) if **to == Type::Void)
}

fn mentions_void(ty: &Type) -> bool {
    match ty {
        Type::Void => true,
        Type::Pointer(to) | Type::Array(to, _) => mentions_void(to),
        _ => false,
    }
}

/// Refuses a `void` pointer behind a pointer or in an array.
fn nested(ty: &Type, at: u32) -> R<()> {
    match ty {
        Type::Pointer(to) if to.is_pointer() && mentions_void(to) => {
            Err(Unsupported::new("a pointer to a pointer", at))
        }
        Type::Array(of, _) if of.is_pointer() && mentions_void(of) => {
            Err(Unsupported::new("a pointer in an array", at))
        }
        Type::Array(of, _) => nested(of, at),
        _ => Ok(()),
    }
}

/// The type as C writes it, for a message.
fn spelling(ty: &Type, records: &[String]) -> String {
    match ty {
        Type::Void => "void".into(),
        Type::Bool => "_Bool".into(),
        Type::Char => "char".into(),
        Type::Int { bits, signed } => {
            let name = match bits {
                8 => "char",
                16 => "short",
                32 => "int",
                _ => "long",
            };
            match (signed, bits) {
                (true, 8) => "signed char".into(),
                (true, _) => name.into(),
                (false, _) => format!("unsigned {name}"),
            }
        }
        Type::Float { bits: 32 } => "float".into(),
        Type::Float { .. } => "double".into(),
        Type::Index => "size_t".into(),
        Type::Found => "a place that a search found".into(),
        Type::Pointer(to) => format!("{} *", spelling(to, records)),
        Type::Array(of, _) => format!("{} []", spelling(of, records)),
        Type::Record(id) => format!("struct {}", records[*id]),
        Type::Function { .. } => "a function".into(),
    }
}

fn article(name: &str) -> &'static str {
    match name.chars().next() {
        Some('a' | 'e' | 'i' | 'o' | 'u') => "an",
        _ => "a",
    }
}

#[derive(Default)]
struct Voids {
    /// The node of each `void` pointer slot. Conversions have nodes of
    /// their own, without a slot.
    nodes: HashMap<Slot, usize>,
    parent: Vec<usize>,
    /// Where each node is first seen, for messages; 0 when unknown.
    at: Vec<u32>,
    /// The pointer types stored into a node or that it is converted to.
    types: Vec<(usize, Type)>,
    /// The one type of each group, by its root, once decided.
    decided: HashMap<usize, Type>,
    functions: HashMap<String, FuncId>,
    params: Vec<Vec<Slot>>,
    function: Option<FuncId>,
    /// Whether the walk rewrites the tree; the first only collects.
    rewriting: bool,
    /// The number of conversions seen in this walk, so that the second
    /// finds the nodes of the first.
    conversions: usize,
    conversion_nodes: Vec<usize>,
    /// The names of the records, for messages.
    names: Vec<String>,
}

impl Voids {
    fn root(&mut self, node: usize) -> usize {
        let mut node = node;
        while self.parent[node] != node {
            self.parent[node] = self.parent[self.parent[node]];
            node = self.parent[node];
        }
        node
    }

    fn join(&mut self, a: usize, b: usize) {
        let (a, b) = (self.root(a), self.root(b));
        if a != b {
            self.parent[b.max(a)] = a.min(b);
        }
    }

    fn node(&self, slot: Slot) -> Option<usize> {
        self.nodes.get(&slot).copied()
    }

    /// The one type of the group the node is in, in the second walk.
    fn group_type(&mut self, node: usize) -> Option<Type> {
        let root = self.root(node);
        self.decided.get(&root).cloned()
    }

    fn slot_type(&mut self, slot: Slot) -> Option<Type> {
        let node = self.node(slot)?;
        self.group_type(node)
    }

    fn walk(&mut self, program: &mut Program) -> R<()> {
        self.conversions = 0;
        for id in 0..program.vars.len() {
            let Some(mut init) = program.vars[id].init.take() else {
                continue;
            };
            let walked = match self.node(Slot::Var(id)) {
                Some(node) => self.flow(&mut init, node),
                None => self.visit(&mut init),
            };
            program.vars[id].init = Some(init);
            walked?;
            if let Some(len) = &mut program.vars[id].len {
                self.visit(len)?;
            }
        }
        for id in 0..program.functions.len() {
            self.function = Some(id);
            let mut body = std::mem::take(&mut program.functions[id].body);
            let walked = self.block(&mut body);
            program.functions[id].body = body;
            walked?;
        }
        self.function = None;
        Ok(())
    }

    /// Decides the one type of each group, or refuses the program.
    fn decide(&mut self) -> R<()> {
        let mut groups: Vec<(usize, Vec<Type>)> = Vec::new();
        for (node, ty) in std::mem::take(&mut self.types) {
            let root = self.root(node);
            match groups.iter_mut().find(|(group, _)| *group == root) {
                Some((_, types)) if !types.contains(&ty) => types.push(ty),
                Some(_) => {}
                None => groups.push((root, vec![ty])),
            }
        }
        let mut slots: Vec<(u32, usize)> = Vec::new();
        for node in self.nodes.values().copied().collect::<Vec<_>>() {
            let root = self.root(node);
            slots.push((self.at[node], root));
        }
        slots.sort();
        for (at, root) in slots {
            let types = groups
                .iter()
                .find(|(group, _)| *group == root)
                .map(|(_, types)| types.as_slice())
                .unwrap_or_default();
            match types {
                [] => {
                    return Err(Unsupported::new(
                        "a `void` pointer whose type nothing says",
                        at,
                    ));
                }
                [_] => {}
                _ => {
                    let names: Vec<String> = types
                        .iter()
                        .map(|ty| {
                            let name = spelling(ty.element().unwrap_or(&Type::Void), &self.names);
                            format!("{} `{name}`", article(&name))
                        })
                        .collect();
                    let (last, first) = names.split_last().unwrap_or((&names[0], &[]));
                    return Err(Unsupported::new(
                        format!(
                            "a `void` pointer that holds {} and {last}",
                            first.join(", ")
                        ),
                        at,
                    ));
                }
            }
        }
        for (root, types) in groups {
            if let [ty] = types.as_slice() {
                self.decided.insert(root, ty.clone());
            }
        }
        Ok(())
    }

    fn block(&mut self, stmts: &mut [Stmt]) -> R<()> {
        for stmt in stmts {
            match stmt {
                Stmt::Expr(expr) => self.visit(expr)?,
                Stmt::Return(Some(expr)) => {
                    match self.function.and_then(|f| self.node(Slot::Return(f))) {
                        Some(node) => self.flow(expr, node)?,
                        None => self.visit(expr)?,
                    }
                }
                Stmt::Block(body) => self.block(body)?,
                Stmt::If(cond, then, other) => {
                    self.visit(cond)?;
                    self.block(then)?;
                    self.block(other)?;
                }
                Stmt::While(cond, body) | Stmt::DoWhile(body, cond) | Stmt::Switch(cond, body) => {
                    self.visit(cond)?;
                    self.block(body)?;
                }
                Stmt::For {
                    init,
                    cond,
                    step,
                    body,
                } => {
                    self.block(init)?;
                    for part in cond.iter_mut().chain(step) {
                        self.visit(part)?;
                    }
                    self.block(body)?;
                }
                Stmt::IfSome {
                    source,
                    then,
                    other,
                    ..
                } => {
                    self.visit(source)?;
                    self.block(then)?;
                    self.block(other)?;
                }
                Stmt::LetSome {
                    source, otherwise, ..
                } => {
                    self.visit(source)?;
                    self.block(otherwise)?;
                }
                Stmt::Parse { call, start, .. } => {
                    self.visit(call)?;
                    if let Some((start, _)) = start {
                        self.visit(start)?;
                    }
                }
                // Initialisers are walked with their variables.
                Stmt::Decl(_)
                | Stmt::Return(None)
                | Stmt::Case(_)
                | Stmt::Default
                | Stmt::Break
                | Stmt::Continue
                | Stmt::Goto(_)
                | Stmt::Label(_) => {}
            }
        }
        Ok(())
    }

    /// The node of a `void` pointer slot that the place is.
    fn place_node(&self, place: &Expr) -> Option<usize> {
        place_slot(place).and_then(|slot| self.node(slot))
    }

    /// Gives the expression the type of its slot's group, in the second
    /// walk.
    fn retype(&mut self, expr: &mut Expr, node: Option<usize>) {
        if self.rewriting
            && is_void(&expr.ty)
            && let Some(ty) = node.and_then(|node| self.group_type(node))
        {
            expr.ty = ty;
        }
    }

    /// An expression whose value does not go into a `void` pointer slot.
    fn visit(&mut self, expr: &mut Expr) -> R<()> {
        let at = expr.at;
        let arithmetic = || {
            Err(Unsupported::new(
                "arithmetic on a `void` pointer: no rule yet",
                at,
            ))
        };
        match &mut expr.kind {
            ExprKind::SizeOf(Type::Void) => {
                return Err(Unsupported::new("the size of `void`", at));
            }
            ExprKind::Binary(BinOp::Add | BinOp::Sub, left, right)
                if is_void(&left.ty) || is_void(&right.ty) =>
            {
                return arithmetic();
            }
            ExprKind::Assign {
                op: Some(_),
                target,
                ..
            }
            | ExprKind::IncDec { target, .. }
                if is_void(&target.ty) =>
            {
                return arithmetic();
            }
            ExprKind::Deref(pointer) | ExprKind::Index(pointer, _) if is_void(&pointer.ty) => {
                return Err(Unsupported::new("a read through a `void` pointer", at));
            }
            ExprKind::AddrOf(place) if is_void(&place.ty) => {
                return Err(Unsupported::new("a pointer to a pointer", at));
            }
            ExprKind::Assign {
                op: None,
                target,
                value,
            } if is_void(&target.ty) => {
                let node = self.place_node(target);
                for child in target.children_mut() {
                    self.visit(child)?;
                }
                match node {
                    Some(node) => self.flow(value, node)?,
                    None => self.visit(value)?,
                }
                self.retype(target, node);
                self.retype(expr, node);
                return Ok(());
            }
            ExprKind::Call(callee, args) => {
                let function = callee
                    .function()
                    .and_then(|name| self.functions.get(name).copied());
                for (index, arg) in args.iter_mut().enumerate() {
                    let param = function
                        .and_then(|f| self.params[f].get(index).copied())
                        .and_then(|slot| self.node(slot));
                    match param {
                        Some(node) => self.flow(arg, node)?,
                        None => self.visit(arg)?,
                    }
                }
                let node = function.and_then(|f| self.node(Slot::Return(f)));
                self.retype(expr, node);
                return Ok(());
            }
            // A conversion of a `void` pointer to the type it points to.
            ExprKind::Cast(inner) if is_void(&inner.ty) && expr.ty.is_pointer() => {
                if matches!(expr.ty.element(), Some(Type::Function { .. })) {
                    return kept_function(at);
                }
                if is_void(&expr.ty) {
                    for child in expr.children_mut() {
                        self.visit(child)?;
                    }
                    return Ok(());
                }
                let node = self.conversion();
                if !self.rewriting {
                    self.types.push((node, expr.ty.clone()));
                }
                self.flow(inner, node)?;
                if self.rewriting && inner.ty == expr.ty {
                    *expr = std::mem::replace(&mut **inner, hole());
                }
                return Ok(());
            }
            ExprKind::InitList(items) => {
                if let Type::Record(record) = expr.ty {
                    for (field, item) in items.iter_mut().enumerate() {
                        match self.node(Slot::Field(record, field)) {
                            Some(node) => self.flow(item, node)?,
                            None => self.visit(item)?,
                        }
                    }
                    return Ok(());
                }
            }
            _ => {}
        }
        for child in expr.children_mut() {
            self.visit(child)?;
        }
        let node = self.place_node(expr);
        self.retype(expr, node);
        Ok(())
    }

    /// The node of the next conversion: new in the first walk, the same
    /// one in the second.
    fn conversion(&mut self) -> usize {
        let index = self.conversions;
        self.conversions += 1;
        if let Some(&node) = self.conversion_nodes.get(index) {
            return node;
        }
        let node = self.parent.len();
        self.parent.push(node);
        self.at.push(0);
        self.conversion_nodes.push(node);
        node
    }

    /// A value that goes into a slot of the node's group.
    fn flow(&mut self, expr: &mut Expr, node: usize) -> R<()> {
        if !is_void(&expr.ty) {
            return self.visit(expr);
        }
        if self.at[node] == 0 {
            self.at[node] = expr.at;
        }
        let at = expr.at;
        match &mut expr.kind {
            ExprKind::Null | ExprKind::Zero => {}
            ExprKind::Cast(inner) => {
                if matches!(inner.uncast().kind, ExprKind::Function(_))
                    || matches!(inner.ty, Type::Function { .. })
                    || matches!(inner.ty.element(), Some(Type::Function { .. }))
                {
                    return kept_function(at);
                }
                if is_void(&inner.ty) {
                    self.flow(inner, node)?;
                } else if inner.ty.is_pointer() {
                    if !self.rewriting {
                        self.types.push((node, inner.ty.clone()));
                    }
                    self.visit(inner)?;
                } else {
                    return self.visit(inner);
                }
                if self.rewriting && self.group_type(node).is_some_and(|ty| ty == inner.ty) {
                    *expr = std::mem::replace(&mut **inner, hole());
                }
            }
            ExprKind::Var(_) | ExprKind::Member { .. } => {
                if let Some(slot) = self.place_node(expr) {
                    self.join(node, slot);
                }
                self.visit(expr)?;
            }
            ExprKind::Cond(cond, then, other) => {
                self.visit(cond)?;
                self.flow(then, node)?;
                self.flow(other, node)?;
                self.retype(expr, Some(node));
            }
            ExprKind::Comma(first, last) => {
                self.visit(first)?;
                self.flow(last, node)?;
                self.retype(expr, Some(node));
            }
            ExprKind::Assign {
                op: None, target, ..
            } => {
                if let Some(slot) = self.place_node(target) {
                    self.join(node, slot);
                }
                self.visit(expr)?;
            }
            ExprKind::Call(callee, _)
                if callee
                    .function()
                    .and_then(|name| self.functions.get(name))
                    .is_some() =>
            {
                let function = callee.function().and_then(|name| self.functions.get(name));
                if let Some(slot) = function.and_then(|&f| self.node(Slot::Return(f))) {
                    self.join(node, slot);
                }
                self.visit(expr)?;
            }
            // Memory from `malloc` or another function of the library: the
            // allocation is of the group's type, which a cast says, as C
            // says it with `(T *)malloc(…)`.
            _ => {
                self.visit(expr)?;
                if self.rewriting
                    && let Some(ty) = self.group_type(node)
                {
                    let value = std::mem::replace(expr, hole());
                    *expr = Expr {
                        ty,
                        kind: ExprKind::Cast(Box::new(value)),
                        at,
                    };
                }
            }
        }
        Ok(())
    }
}

fn kept_function(at: u32) -> R<()> {
    Err(Unsupported::new(
        "a function kept in a `void` pointer: no rule yet",
        at,
    ))
}

/// What stands where an expression has been taken out.
fn hole() -> Expr {
    Expr {
        ty: Type::Void,
        kind: ExprKind::Null,
        at: 0,
    }
}
