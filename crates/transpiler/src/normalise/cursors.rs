//! `cursors`, R24: a pointer that walks an array becomes an index beside
//! the array. C moves the pointer (`p++`) and compares it with another
//! (`p < end`); Rust keeps the slice where it is and counts in a `usize`.
//!
//! The rewrite works on one function at a time. It finds the variables that
//! need an index, puts those that are offsets of one another into a group,
//! and looks for the one array that a group walks, its *base*. A group
//! without one stays as it is, and the rules refuse what they have no
//! translation for.

use crate::c::{BinOp, Expr, ExprKind, Program, Stmt, Type, UnOp, Var, VarId, VarKind};
use crate::facts::{Facts, Slot};
use std::collections::{HashMap, HashSet};

pub fn cursors(program: &mut Program, facts: &Facts) {
    for function in 0..program.functions.len() {
        let mut body = std::mem::take(&mut program.functions[function].body);
        let params = program.functions[function].params.clone();
        let scan = Scan::of(program, facts, &params, &body);
        let mut groups = scan.groups();
        let taken = [scan.declared.as_slice(), program.globals.as_slice()].concat();
        // Every value of a cursor must have an index in its array. A group
        // with one that has none is no group of cursors.
        loop {
            let mut rewrite = Rewrite::new(program, facts, groups.clone(), &taken);
            let without: HashSet<&String> = groups
                .iter()
                .filter(|(cursor, (key, _))| {
                    let values = scan.values.get(cursor).into_iter().flatten();
                    values
                        .chain(scan.moves.get(cursor).into_iter().flatten())
                        .any(|value| rewrite.index_of(value, key).is_none())
                })
                .map(|(_, (key, _))| key)
                .collect();
            if without.is_empty() {
                break;
            }
            let without: HashSet<String> = without.into_iter().cloned().collect();
            groups.retain(|_, (key, _)| !without.contains(key));
        }
        if !groups.is_empty() {
            let mut rewrite = Rewrite::new(program, facts, groups, &taken);
            rewrite.stmts(&mut body);
            let Rewrite { added, changed, .. } = rewrite;
            for (var, changed) in changed {
                program.vars[var] = changed;
            }
            // A parameter stays the slice, and its index starts at zero.
            for (position, index) in added.into_iter().enumerate() {
                body.insert(position, Stmt::Decl(program.vars.len()));
                program.vars.push(index);
            }
        }
        program.functions[function].body = body;
    }
}

/// The expression at the root of pointer arithmetic: `a` in `&a[i] + 1`.
/// An assignment and `p++` have the value of their target.
fn root(pointer: &Expr) -> &Expr {
    match &pointer.kind {
        ExprKind::Cast(inner) if inner.ty == pointer.ty => root(inner),
        ExprKind::Binary(BinOp::Add | BinOp::Sub, left, right) => {
            match (left.ty.is_pointer(), right.ty.is_pointer()) {
                (true, false) => root(left),
                (false, true) => root(right),
                _ => pointer,
            }
        }
        ExprKind::AddrOf(place) => match &place.kind {
            ExprKind::Index(base, _) => root(base),
            _ => pointer,
        },
        ExprKind::IncDec { target, .. } | ExprKind::Assign { target, .. }
            if matches!(target.kind, ExprKind::Var(_)) =>
        {
            target
        }
        _ => pointer,
    }
}

fn root_var(pointer: &Expr) -> Option<VarId> {
    match root(pointer).kind {
        ExprKind::Var(var) => Some(var),
        _ => None,
    }
}

/// What a function does with its pointers, as far as cursors go.
struct Scan<'a> {
    program: &'a Program,
    facts: &'a Facts,
    /// The pointers that `++`, `--`, `+=`, `-=` or `p = p + n` move.
    moved: HashSet<VarId>,
    /// The pointers at the root of a side of a comparison or a difference
    /// of two pointers.
    compared: HashSet<VarId>,
    /// The values each pointer variable gets, apart from its own moves.
    values: HashMap<VarId, Vec<&'a Expr>>,
    /// The expressions that move each pointer variable.
    moves: HashMap<VarId, Vec<&'a Expr>>,
    /// The locals in scope where each local is declared.
    visible: HashMap<VarId, Vec<VarId>>,
    scope: Vec<VarId>,
    /// The function's parameters and all of its locals.
    declared: Vec<VarId>,
}

/// The cursors of a function: for each, its base with the base's key.
type Groups = HashMap<VarId, (String, Expr)>;

impl<'a> Scan<'a> {
    fn of(program: &'a Program, facts: &'a Facts, params: &[VarId], body: &'a [Stmt]) -> Self {
        let mut scan = Scan {
            program,
            facts,
            moved: HashSet::new(),
            compared: HashSet::new(),
            values: HashMap::new(),
            moves: HashMap::new(),
            visible: HashMap::new(),
            scope: Vec::new(),
            declared: params.to_vec(),
        };
        scan.block(body);
        scan
    }

    fn block(&mut self, stmts: &'a [Stmt]) {
        let outer = self.scope.len();
        for stmt in stmts {
            self.stmt(stmt);
        }
        self.scope.truncate(outer);
    }

    fn stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            Stmt::Expr(expr) | Stmt::Return(Some(expr)) => self.expr(expr),
            Stmt::Decl(var) => {
                let declared = &self.program.vars[*var];
                for part in declared.init.iter().chain(&declared.len) {
                    self.expr(part);
                }
                if let (true, Some(init)) = (declared.ty.is_pointer(), &declared.init) {
                    self.values.entry(*var).or_default().push(init);
                }
                self.visible.insert(*var, self.scope.clone());
                self.scope.push(*var);
                self.declared.push(*var);
            }
            Stmt::Block(body) => self.block(body),
            Stmt::If(cond, then, other) => {
                self.expr(cond);
                self.block(then);
                self.block(other);
            }
            Stmt::While(cond, body) | Stmt::DoWhile(body, cond) | Stmt::Switch(cond, body) => {
                self.expr(cond);
                self.block(body);
            }
            Stmt::For {
                init,
                cond,
                step,
                body,
            } => {
                // What the loop declares ends with it.
                let outer = self.scope.len();
                for stmt in init {
                    self.stmt(stmt);
                }
                for part in cond.iter().chain(step) {
                    self.expr(part);
                }
                self.block(body);
                self.scope.truncate(outer);
            }
            Stmt::IfSome {
                source,
                then,
                other,
                ..
            } => {
                self.expr(source);
                self.block(then);
                self.block(other);
            }
            Stmt::LetSome {
                source, otherwise, ..
            } => {
                self.expr(source);
                self.block(otherwise);
            }
            Stmt::Return(None)
            | Stmt::Case(_)
            | Stmt::Default
            | Stmt::Break
            | Stmt::Continue
            | Stmt::Goto(_)
            | Stmt::Label(_) => {}
        }
    }

    fn expr(&mut self, expr: &'a Expr) {
        let pointer_var = |place: &Expr| match place.kind {
            ExprKind::Var(var) if place.ty.is_pointer() => Some(var),
            _ => None,
        };
        match &expr.kind {
            ExprKind::IncDec { target, .. } => {
                if let Some(var) = pointer_var(target) {
                    self.moved.insert(var);
                    self.moves.entry(var).or_default().push(expr);
                }
            }
            ExprKind::Assign { op, target, value } => {
                if let Some(var) = pointer_var(target) {
                    match op.is_some() || root_var(value) == Some(var) {
                        true => {
                            self.moved.insert(var);
                            self.moves.entry(var).or_default().push(expr);
                        }
                        false => self.values.entry(var).or_default().push(value),
                    }
                }
            }
            ExprKind::Binary(op, left, right)
                if (op.is_comparison() || *op == BinOp::Sub)
                    && left.ty.is_pointer()
                    && right.ty.is_pointer() =>
            {
                let null = |side: &Expr| matches!(side.uncast().kind, ExprKind::Null);
                if !null(left) && !null(right) {
                    self.compared.extend(root_var(left));
                    self.compared.extend(root_var(right));
                }
            }
            _ => {}
        }
        for child in expr.children() {
            self.expr(child);
        }
    }

    /// The variables that become indices, each with the base of its group.
    fn groups(&self) -> Groups {
        let vars = &self.program.vars;
        let fits = |var: VarId| {
            let declared = &vars[var];
            matches!(declared.kind, VarKind::Local | VarKind::Param)
                && !self.facts.addressed(var)
                && !matches!(
                    declared.ty.element(),
                    None | Some(Type::Void | Type::Function { .. })
                )
        };
        let no_params = HashSet::new();
        // A local whose every value is an offset of a variable or of an
        // array. One that gets memory from elsewhere, such as `malloc`, is
        // no cursor, and may be a base.
        let derived = |var: VarId| {
            vars[var].kind == VarKind::Local
                && fits(var)
                && self.values.get(&var).is_some_and(|values| {
                    values.iter().all(|value| {
                        let from = root(value);
                        matches!(from.kind, ExprKind::Var(_))
                            || self.stable(from, &no_params).is_some()
                    })
                })
        };
        // A pointer that is only compared needs an index if it is such a
        // local. Text that is compared keeps its own rule.
        let compared = |var: VarId| {
            let pointer = self.facts.pointer(Slot::Var(var));
            self.compared.contains(&var)
                && derived(var)
                && (pointer.bytes || vars[var].ty.element() != Some(&Type::Char))
        };
        let mut members: Vec<VarId> = (0..vars.len())
            .filter(|var| fits(*var) && (self.moved.contains(var) || compared(*var)))
            .collect();
        // So does a local that a member gets a value from: `q` in
        // `q = p + 2; p = q;`.
        let mut next = 0;
        while let Some(&member) = members.get(next) {
            next += 1;
            for value in self.values.get(&member).into_iter().flatten() {
                if let Some(from) = root_var(value)
                    && !members.contains(&from)
                    && derived(from)
                {
                    members.push(from);
                }
            }
        }
        // The group of each member is that of the first member it reaches.
        let mut group: HashMap<VarId, VarId> = members.iter().map(|&var| (var, var)).collect();
        let mut bases: HashMap<VarId, Vec<(String, Expr)>> = HashMap::new();
        let mut failed: HashSet<VarId> = HashSet::new();
        let moved_params: HashSet<VarId> = members
            .iter()
            .copied()
            .filter(|var| vars[*var].kind == VarKind::Param)
            .collect();
        for &member in &members {
            if vars[member].kind == VarKind::Param {
                let own = Expr {
                    ty: vars[member].ty.clone(),
                    kind: ExprKind::Var(member),
                    at: vars[member].at,
                };
                bases
                    .entry(member)
                    .or_default()
                    .push((format!("v{member}"), own));
            }
            for value in self.values.get(&member).into_iter().flatten() {
                let from = root(value);
                match from.kind {
                    ExprKind::Var(other) if group.contains_key(&other) => {
                        let (old, new) = (group[&member], group[&other]);
                        for joined in group.values_mut().filter(|joined| **joined == old) {
                            *joined = new;
                        }
                    }
                    _ => match self.stable(from, &moved_params) {
                        Some(key) if from.ty == vars[member].ty => {
                            bases.entry(member).or_default().push((key, from.clone()))
                        }
                        _ => {
                            failed.insert(member);
                        }
                    },
                }
            }
        }
        let mut groups = Groups::new();
        for &leader in group.values().collect::<HashSet<_>>() {
            let of_group = || members.iter().copied().filter(|var| group[var] == leader);
            let mut found: Vec<&(String, Expr)> = of_group()
                .flat_map(|var| bases.get(&var))
                .flatten()
                .collect();
            found.dedup_by(|a, b| a.0 == b.0);
            let [(key, base)] = found.as_slice() else {
                continue;
            };
            // The base is named wherever a member is used, so it must be
            // declared before each of them.
            let in_scope = |member: VarId| match (vars[member].kind, base_var(base)) {
                (_, None) | (VarKind::Param, _) => true,
                (_, Some(holder)) => {
                    vars[holder].kind != VarKind::Local
                        || self
                            .visible
                            .get(&member)
                            .is_some_and(|visible| visible.contains(&holder))
                }
            };
            // Nor may another variable of the function have the name of
            // the variable that holds the base: where it hides that one,
            // the name would mean other memory.
            let hidden = base_var(base).is_some_and(|holder| {
                self.declared
                    .iter()
                    .any(|&other| other != holder && vars[other].name == vars[holder].name)
            });
            if hidden || of_group().any(|var| failed.contains(&var) || !in_scope(var)) {
                continue;
            }
            for member in of_group() {
                groups.insert(member, (key.clone(), base.clone()));
            }
        }
        groups
    }

    /// A name for the memory that a base points to, if the expression means
    /// the same memory wherever the function evaluates it.
    fn stable(&self, pointer: &Expr, moved_params: &HashSet<VarId>) -> Option<String> {
        stable(self.program, self.facts, pointer, moved_params)
    }
}

/// The variable that holds a base.
fn base_var(base: &Expr) -> Option<VarId> {
    match &base.kind {
        ExprKind::Var(var) => Some(*var),
        ExprKind::Decay(inner)
        | ExprKind::Cast(inner)
        | ExprKind::Member { base: inner, .. }
        | ExprKind::Index(inner, _) => base_var(inner),
        _ => None,
    }
}

fn stable(
    program: &Program,
    facts: &Facts,
    pointer: &Expr,
    moved_params: &HashSet<VarId>,
) -> Option<String> {
    match &pointer.kind {
        // A pointer that keeps its first value. A parameter that only its
        // own moves change keeps it once they move its index.
        ExprKind::Var(var) if pointer.ty.is_pointer() => {
            (!facts.mutated(*var) || moved_params.contains(var)).then(|| format!("v{var}"))
        }
        ExprKind::Cast(inner) if inner.ty == pointer.ty => {
            stable(program, facts, inner, moved_params)
        }
        ExprKind::Decay(array) => place(program, facts, array, moved_params),
        _ => None,
    }
}

/// A name for a place that is always the same memory: a variable, a field
/// of one, or an element at a constant index.
fn place(
    program: &Program,
    facts: &Facts,
    place_: &Expr,
    moved_params: &HashSet<VarId>,
) -> Option<String> {
    match &place_.kind {
        ExprKind::Var(var) if !place_.ty.is_pointer() => Some(format!("v{var}")),
        ExprKind::Member {
            base,
            record,
            field,
            arrow,
        } => {
            let holder = match arrow {
                true => stable(program, facts, base, moved_params)?,
                false => place(program, facts, base, moved_params)?,
            };
            Some(format!("{holder}.{record}.{field}"))
        }
        ExprKind::Index(base, index) => {
            let ExprKind::Decay(array) = &base.kind else {
                return None;
            };
            let index = index.constant(program)?;
            Some(format!(
                "{}[{index}]",
                place(program, facts, array, moved_params)?
            ))
        }
        _ => None,
    }
}

struct Rewrite<'a> {
    program: &'a Program,
    facts: &'a Facts,
    groups: Groups,
    /// The variable that holds each cursor's index: the cursor itself for
    /// a local, and a new local for a parameter.
    index: HashMap<VarId, VarId>,
    /// The new locals, in the order of their numbers.
    added: Vec<Var>,
    /// The variables that change their type or their first value.
    changed: HashMap<VarId, Var>,
    moved_params: HashSet<VarId>,
}

impl<'a> Rewrite<'a> {
    /// `taken` are the variables in whose scope the function is: its own
    /// and the globals.
    fn new(program: &'a Program, facts: &'a Facts, groups: Groups, taken: &[VarId]) -> Self {
        let mut rewrite = Rewrite {
            program,
            facts,
            index: HashMap::new(),
            added: Vec::new(),
            changed: HashMap::new(),
            moved_params: HashSet::new(),
            groups,
        };
        let mut cursors: Vec<VarId> = rewrite.groups.keys().copied().collect();
        cursors.sort_unstable();
        for cursor in cursors {
            let var = &program.vars[cursor];
            if var.kind == VarKind::Param {
                rewrite.moved_params.insert(cursor);
                rewrite
                    .index
                    .insert(cursor, program.vars.len() + rewrite.added.len());
                // A name that no variable has where the index is in scope.
                let mut name = format!("{}_at", var.name);
                while taken.iter().any(|&other| program.vars[other].name == name) {
                    name.push('_');
                }
                rewrite.added.push(Var {
                    name,
                    ty: Type::Index,
                    init: Some(index_literal(0, var.at)),
                    len: None,
                    kind: VarKind::Local,
                    at: var.at,
                });
            } else {
                rewrite.index.insert(cursor, cursor);
            }
        }
        rewrite
    }

    fn stmts(&mut self, stmts: &mut [Stmt]) {
        for stmt in stmts {
            self.stmt(stmt);
        }
    }

    fn stmt(&mut self, stmt: &mut Stmt) {
        match stmt {
            Stmt::Expr(expr) => self.effect(expr),
            Stmt::Return(Some(expr)) => self.expr(expr),
            Stmt::Decl(var) => {
                let mut declared = self.program.vars[*var].clone();
                let cursor = self.groups.get(var).map(|(key, _)| key.clone());
                if let Some(init) = &mut declared.init {
                    // `cursors` found an index for every value of a cursor.
                    match cursor.as_ref().and_then(|key| self.index_of(init, key)) {
                        Some(index) => *init = index,
                        None => self.expr(init),
                    }
                }
                if let Some(len) = &mut declared.len {
                    self.expr(len);
                }
                if cursor.is_some() {
                    declared.ty = Type::Index;
                }
                self.changed.insert(*var, declared);
            }
            Stmt::Block(body) => self.stmts(body),
            Stmt::If(cond, then, other) => {
                self.test(cond);
                self.stmts(then);
                self.stmts(other);
            }
            Stmt::While(cond, body) | Stmt::DoWhile(body, cond) => {
                self.test(cond);
                self.stmts(body);
            }
            Stmt::Switch(value, body) => {
                self.expr(value);
                self.stmts(body);
            }
            Stmt::For {
                init,
                cond,
                step,
                body,
            } => {
                self.stmts(init);
                if let Some(cond) = cond {
                    self.test(cond);
                }
                if let Some(step) = step {
                    self.effect(step);
                }
                self.stmts(body);
            }
            Stmt::IfSome {
                source,
                then,
                other,
                ..
            } => {
                self.expr(source);
                self.stmts(then);
                self.stmts(other);
            }
            Stmt::LetSome {
                source, otherwise, ..
            } => {
                self.expr(source);
                self.stmts(otherwise);
            }
            Stmt::Return(None)
            | Stmt::Case(_)
            | Stmt::Default
            | Stmt::Break
            | Stmt::Continue
            | Stmt::Goto(_)
            | Stmt::Label(_) => {}
        }
    }

    /// The cursor at the root of a pointer expression.
    fn cursor_of(&self, pointer: &Expr) -> Option<VarId> {
        root_var(pointer).filter(|var| pointer.ty.is_pointer() && self.groups.contains_key(var))
    }

    /// The base and the index of a pointer expression that is an offset of
    /// a cursor.
    fn cursor(&mut self, pointer: &Expr) -> Option<(Expr, Expr)> {
        let (key, base) = self.groups.get(&self.cursor_of(pointer)?)?.clone();
        Some((base, self.index_of(pointer, &key)?))
    }

    /// The index of a pointer expression in the array with this key, if the
    /// expression is an offset of the array or of one of its cursors.
    fn index_of(&mut self, pointer: &Expr, key: &str) -> Option<Expr> {
        let at = pointer.at;
        let var = |var: VarId| Expr {
            ty: Type::Index,
            kind: ExprKind::Var(var),
            at,
        };
        let cursor = |rewrite: &Self, place: &Expr| match place.kind {
            ExprKind::Var(cursor)
                if rewrite
                    .groups
                    .get(&cursor)
                    .is_some_and(|(found, _)| found == key) =>
            {
                Some(rewrite.index[&cursor])
            }
            _ => None,
        };
        if !pointer.ty.is_pointer() {
            return None;
        }
        match &pointer.kind {
            ExprKind::Var(_) if cursor(self, pointer).is_some() => cursor(self, pointer).map(var),
            ExprKind::Cast(inner) if inner.ty == pointer.ty => self.index_of(inner, key),
            ExprKind::Binary(op @ (BinOp::Add | BinOp::Sub), left, right) => {
                let (from, offset) = match (left.ty.is_pointer(), right.ty.is_pointer()) {
                    (true, false) => (left, right),
                    (false, true) if *op == BinOp::Add => (right, left),
                    _ => return None,
                };
                let from = self.index_of(from, key)?;
                Some(self.offset(from, *op, offset))
            }
            ExprKind::AddrOf(place) => match &place.kind {
                ExprKind::Index(base, offset) => {
                    let from = self.index_of(base, key)?;
                    Some(self.offset(from, BinOp::Add, offset))
                }
                _ => None,
            },
            ExprKind::IncDec {
                target,
                increment,
                prefix,
            } => Some(Expr {
                ty: Type::Index,
                kind: ExprKind::IncDec {
                    target: Box::new(var(cursor(self, target)?)),
                    increment: *increment,
                    prefix: *prefix,
                },
                at,
            }),
            ExprKind::Assign { op, target, value } => {
                let target = Box::new(var(cursor(self, target)?));
                let (op, value) = match op {
                    None => (None, self.index_of(value, key)?),
                    Some((op @ (BinOp::Add | BinOp::Sub), _)) => {
                        let mut value = (**value).clone();
                        self.expr(&mut value);
                        (Some((*op, Type::Index)), as_index(&value, self.program))
                    }
                    Some(_) => return None,
                };
                Some(Expr {
                    ty: Type::Index,
                    kind: ExprKind::Assign {
                        op,
                        target,
                        value: Box::new(value),
                    },
                    at,
                })
            }
            // The array itself is its own start.
            _ => (stable(self.program, self.facts, pointer, &self.moved_params).as_deref()
                == Some(key))
            .then(|| index_literal(0, at)),
        }
    }

    /// `index + offset` or `index - offset`, where the offset is a number
    /// of the C program.
    fn offset(&mut self, index: Expr, op: BinOp, offset: &Expr) -> Expr {
        let at = offset.at;
        let mut offset = offset.clone();
        self.expr(&mut offset);
        let known = |expr: &Expr| match expr.kind {
            ExprKind::Int(value) => Some(value),
            _ => None,
        };
        let binary = |op: BinOp, left: Expr, right: Expr| Expr {
            ty: Type::Index,
            kind: ExprKind::Binary(op, Box::new(left), Box::new(right)),
            at,
        };
        match offset.constant(self.program) {
            Some(constant) => {
                let step = if op == BinOp::Sub {
                    -constant
                } else {
                    constant
                };
                match (known(&index), step) {
                    (_, 0) => index,
                    (Some(start), _) if start + step >= 0 => index_literal(start + step, at),
                    (_, 1..) => binary(BinOp::Add, index, index_literal(step, at)),
                    _ => binary(BinOp::Sub, index, index_literal(-step, at)),
                }
            }
            None => {
                let offset = as_index(&offset, self.program);
                match (known(&index), op) {
                    (Some(0), BinOp::Add) => offset,
                    _ => binary(op, index, offset),
                }
            }
        }
    }

    /// A pointer where C asks whether it is NULL: an offset of an array is
    /// NULL only where the array is.
    fn test(&mut self, cond: &mut Expr) {
        if cond.ty.is_pointer()
            && cond.is_pure()
            && let Some((base, _)) = self.cursor(cond)
        {
            *cond = base;
            return;
        }
        self.expr(cond);
    }

    /// An expression that is evaluated for what it does: a move of a
    /// cursor is a move of its index.
    fn effect(&mut self, expr: &mut Expr) {
        match &mut expr.kind {
            ExprKind::Comma(first, last) => {
                self.effect(first);
                self.effect(last);
            }
            ExprKind::Cast(inner) if expr.ty == Type::Void => self.effect(inner),
            ExprKind::IncDec { .. } | ExprKind::Assign { .. } => {
                let moved = self
                    .cursor_of(expr)
                    .and_then(|cursor| self.groups.get(&cursor))
                    .map(|(key, _)| key.clone())
                    .and_then(|key| self.index_of(expr, &key));
                match moved {
                    Some(moved) => *expr = moved,
                    None => self.expr(expr),
                }
            }
            _ => self.expr(expr),
        }
    }

    fn expr(&mut self, expr: &mut Expr) {
        let at = expr.at;
        let element = |base: &Expr| base.ty.element().cloned().unwrap_or(Type::Void);
        let indexed = |base: Expr, index: Expr| Expr {
            ty: element(&base),
            kind: ExprKind::Index(Box::new(base), Box::new(index)),
            at,
        };
        let null = |side: &Expr| matches!(side.uncast().kind, ExprKind::Null);
        let replacement = match &mut expr.kind {
            ExprKind::Deref(pointer) => self
                .cursor(pointer)
                .map(|(base, index)| indexed(base, index).kind),
            ExprKind::Index(pointer, offset) => self.cursor(pointer).map(|(base, index)| {
                let index = self.offset(index, BinOp::Add, offset);
                indexed(base, index).kind
            }),
            ExprKind::Member {
                base: pointer,
                record,
                field,
                arrow: true,
            } => self.cursor(pointer).map(|(base, index)| ExprKind::Member {
                base: Box::new(indexed(base, index)),
                record: *record,
                field: *field,
                arrow: false,
            }),
            ExprKind::Binary(op, left, right)
                if (op.is_comparison() || *op == BinOp::Sub)
                    && left.ty.is_pointer()
                    && right.ty.is_pointer() =>
            {
                if null(left) || null(right) {
                    self.test(left);
                    self.test(right);
                    return;
                }
                let key = self
                    .cursor_of(left)
                    .or(self.cursor_of(right))
                    .and_then(|cursor| self.groups.get(&cursor))
                    .map(|(key, _)| key.clone());
                let sides = key.and_then(|key| {
                    Some((self.index_of(left, &key)?, self.index_of(right, &key)?))
                });
                sides.map(|(left, right)| {
                    // A difference may be negative, and an index may not.
                    let side = |index: Expr| match op {
                        BinOp::Sub => Expr {
                            ty: expr.ty.clone(),
                            kind: ExprKind::Cast(Box::new(index)),
                            at,
                        },
                        _ => index,
                    };
                    match (*op, &right.kind) {
                        (BinOp::Sub, ExprKind::Int(0)) => side(left).kind,
                        _ => ExprKind::Binary(*op, Box::new(side(left)), Box::new(side(right))),
                    }
                })
            }
            ExprKind::Unary(UnOp::Not, inner) => {
                self.test(inner);
                return;
            }
            ExprKind::Binary(BinOp::And | BinOp::Or, left, right) => {
                self.test(left);
                self.test(right);
                return;
            }
            ExprKind::Cond(cond, then, other) => {
                self.test(cond);
                self.expr(then);
                self.expr(other);
                return;
            }
            ExprKind::Cast(inner) if expr.ty == Type::Bool => {
                self.test(inner);
                return;
            }
            _ => None,
        };
        if let Some(kind) = replacement {
            expr.kind = kind;
            return;
        }
        // A cursor where the program needs the pointer itself.
        if let Some((base, index)) = self.cursor(expr) {
            expr.kind = ExprKind::AddrOf(Box::new(indexed(base, index)));
            return;
        }
        for child in expr.children_mut() {
            self.expr(child);
        }
    }
}

fn index_literal(value: i128, at: u32) -> Expr {
    Expr {
        ty: Type::Index,
        kind: ExprKind::Int(value),
        at,
    }
}

/// A number of the C program as an index. C widens an offset to 64 bits
/// first, which the conversion to an index does as well.
fn as_index(number: &Expr, program: &Program) -> Expr {
    if let Some(value @ 0..) = number.constant(program) {
        return index_literal(value, number.at);
    }
    let number = match &number.kind {
        ExprKind::Cast(inner)
            if matches!(number.ty.int(), Some((64, _))) && inner.ty.int().is_some() =>
        {
            inner
        }
        _ => number,
    };
    if number.ty == Type::Index {
        return number.clone();
    }
    Expr {
        ty: Type::Index,
        kind: ExprKind::Cast(Box::new(number.clone())),
        at: number.at,
    }
}
