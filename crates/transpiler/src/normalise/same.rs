//! `same`, R53: a call that hands one pointer in several arguments to a
//! function that stores through one of them calls a copy of the function in
//! which those parameters are one. Inside the function they are the same
//! memory, and the copy says so; Rust lends memory once to whoever stores
//! there, so the call as written has no translation.
//!
//! The arguments are the same pointer when they read one variable, an
//! array or the address of a variable, under any conversions, and no
//! argument of the call has a side effect. A group is merged only where the
//! lowering would refuse it, a parameter of the group being stored through
//! or owned, where its parameters have one type, and where the function
//! neither assigns nor takes the address of any of them: two parameters
//! that move apart are not one variable. Any other call is left as it is.

use crate::c::{
    Expr, ExprKind, FuncId, Function, Program, Stmt, Type, VarId, any_expr, each_expr_mut,
};
use crate::facts::{Facts, Slot};
use std::collections::{HashMap, HashSet};

/// What the stage knows of a parameter of a function or of a copy.
#[derive(Clone)]
struct Param {
    ty: Type,
    /// It is stored through or owns its memory, so that the lowering
    /// refuses a call that lends its memory to another parameter as well.
    stores: bool,
    /// It is assigned or its address is taken, so that it may stop being
    /// the pointer it was handed.
    changes: bool,
}

/// A function or a copy, as calls of it see it.
struct Callee {
    name: String,
    variadic: bool,
    params: Vec<Param>,
}

struct Copies {
    callees: Vec<Callee>,
    /// The copy of a function for each grouping of its parameters: for
    /// each parameter, the first one of its group.
    copies: HashMap<(FuncId, Vec<usize>), FuncId>,
    /// The copies that a body calls and that do not exist yet.
    pending: Vec<(FuncId, Vec<usize>)>,
    /// The names that a copy must not take.
    taken: HashSet<String>,
}

/// Rewrites the calls; returns whether it changed the program.
pub fn same(program: &mut Program, facts: &Facts) -> bool {
    let vars = &program.vars;
    let callees = program
        .functions
        .iter()
        .map(|function| Callee {
            name: function.name.clone(),
            variadic: function.variadic,
            params: function
                .params
                .iter()
                .map(|&var| {
                    let pointer = facts.pointer(Slot::Var(var));
                    Param {
                        ty: vars[var].ty.clone(),
                        stores: pointer.written || pointer.owned || pointer.environment,
                        changes: facts.mutated(var) || facts.addressed(var),
                    }
                })
                .collect(),
        })
        .collect();
    let mut taken: HashSet<String> = program.functions.iter().map(|f| f.name.clone()).collect();
    taken.extend(program.globals.iter().map(|&var| vars[var].name.clone()));
    for function in &program.functions {
        any_expr(&function.body, vars, &mut |expr| {
            if let ExprKind::Function(name) = &expr.kind {
                taken.insert(name.clone());
            }
            false
        });
    }
    let mut copies = Copies {
        callees,
        copies: HashMap::new(),
        pending: Vec::new(),
        taken,
    };
    // A copy is appended and rewritten in its turn, so that a call in it
    // of itself reaches the copy that exists.
    let mut function = 0;
    while function < program.functions.len() {
        let mut body = std::mem::take(&mut program.functions[function].body);
        each_expr_mut(&mut body, &mut program.vars, &mut |expr| {
            copies.rewrite(expr)
        });
        program.functions[function].body = body;
        for (of, groups) in std::mem::take(&mut copies.pending) {
            let copy = copy(
                program,
                of,
                &groups,
                &copies.callees[program.functions.len()].name,
            );
            if program.searches.contains(&program.functions[of].name) {
                program.searches.push(copy.name.clone());
            }
            program.functions.push(copy);
        }
        function += 1;
    }
    !copies.copies.is_empty()
}

impl Copies {
    /// Makes a call that lends one pointer twice a call of a copy.
    fn rewrite(&mut self, expr: &mut Expr) {
        let ExprKind::Call(callee, args) = &mut expr.kind else {
            return;
        };
        let Some(id) = callee
            .function()
            .and_then(|name| self.callees.iter().position(|c| c.name == name))
        else {
            return;
        };
        let function = &self.callees[id];
        if function.name == "main"
            || function.variadic
            || function.params.len() != args.len()
            || !args.iter().all(Expr::is_pure)
        {
            return;
        }
        let params = &function.params;
        // R65: a pointer to a pointer lends the place it names, which one
        // call lends once; the lowering refuses it.
        let lent = |param: &Param| {
            param.ty.is_pointer()
                && param.ty.function().is_none()
                && !crate::facts::to_pointer(&param.ty)
        };
        let mut groups: Vec<usize> = (0..args.len()).collect();
        for later in 0..args.len() {
            groups[later] = (0..later)
                .find(|&first| {
                    groups[first] == first
                        && lent(&params[first])
                        && lent(&params[later])
                        && same_pointer(&args[first], &args[later])
                })
                .unwrap_or(later);
        }
        // A group that the lowering translates, or that is not one
        // variable in the function, stays as it is.
        for first in 0..args.len() {
            let members: Vec<usize> = (0..args.len()).filter(|&p| groups[p] == first).collect();
            let merged = members.iter().any(|&p| params[p].stores)
                && members
                    .iter()
                    .all(|&p| !params[p].changes && params[p].ty == params[first].ty);
            if !merged {
                for p in members {
                    groups[p] = p;
                }
            }
        }
        if groups.iter().enumerate().all(|(p, &first)| p == first) {
            return;
        }
        let copy = match self.copies.get(&(id, groups.clone())) {
            Some(&copy) => copy,
            None => {
                let copy = self.callees.len();
                let params = (0..args.len())
                    .filter(|&p| groups[p] == p)
                    .map(|first| {
                        let members = (0..args.len()).filter(|&p| groups[p] == first);
                        let stores = members.into_iter().any(|p| params[p].stores);
                        Param {
                            stores,
                            ..params[first].clone()
                        }
                    })
                    .collect();
                let name = function.name.clone();
                let name = self.name(&name);
                self.callees.push(Callee {
                    name,
                    variadic: false,
                    params,
                });
                self.copies.insert((id, groups.clone()), copy);
                self.pending.push((id, groups.clone()));
                copy
            }
        };
        let kept = |p: &usize| groups[*p] == *p;
        if let Type::Function { params, .. } = &mut callee.ty {
            let mut p = 0..;
            params.retain(|_| kept(&p.next().unwrap_or_default()));
        }
        callee.kind = ExprKind::Function(self.callees[copy].name.clone());
        let mut p = 0..;
        args.retain(|_| kept(&p.next().unwrap_or_default()));
    }

    /// A name for a copy of the function that no function, global or other
    /// copy has.
    fn name(&mut self, function: &str) -> String {
        let mut name = format!("{function}_same");
        let mut count = 1;
        while self.taken.contains(&name) {
            count += 1;
            name = format!("{function}_same{count}");
        }
        self.taken.insert(name.clone());
        name
    }
}

/// Whether two arguments are the same pointer: reads of one variable, one
/// array or the address of one variable, under any conversions.
fn same_pointer(left: &Expr, right: &Expr) -> bool {
    fn read(expr: &Expr) -> Option<(u8, VarId)> {
        match &expr.uncast().kind {
            ExprKind::Var(var) => Some((0, *var)),
            ExprKind::Decay(inner) => match inner.kind {
                ExprKind::Var(var) => Some((1, var)),
                _ => None,
            },
            ExprKind::AddrOf(inner) => match inner.kind {
                ExprKind::Var(var) => Some((2, var)),
                _ => None,
            },
            _ => None,
        }
    }
    read(left).is_some_and(|left| read(right) == Some(left))
}

/// The copy of the function in which each parameter is the first of its
/// group, with variables of its own for its parameters and locals; a
/// `static` local stays the one variable it is.
fn copy(program: &mut Program, of: FuncId, groups: &[usize], name: &str) -> Function {
    let function = &program.functions[of];
    let mut own: Vec<VarId> = function.params.clone();
    let note = |var: VarId, own: &mut Vec<VarId>| {
        if !program.vars[var].kind.is_global() && !own.contains(&var) {
            own.push(var);
        }
    };
    declared(&function.body, &mut |var| note(var, &mut own));
    let mut read = Vec::new();
    any_expr(&function.body, &program.vars, &mut |expr| {
        if let ExprKind::Var(var) = expr.kind {
            read.push(var);
        }
        false
    });
    for var in read {
        note(var, &mut own);
    }
    let mut fresh: HashMap<VarId, VarId> = HashMap::new();
    for &var in &own {
        fresh.insert(var, program.vars.len());
        program.vars.push(program.vars[var].clone());
    }
    let function = &program.functions[of];
    for (p, &first) in groups.iter().enumerate() {
        fresh.insert(function.params[p], fresh[&function.params[first]]);
    }
    let params = (0..groups.len())
        .filter(|&p| groups[p] == p)
        .map(|p| fresh[&function.params[p]])
        .collect();
    let mut body = function.body.clone();
    let copy = Function {
        name: name.to_owned(),
        ret: function.ret.clone(),
        params,
        variadic: function.variadic,
        body: Vec::new(),
    };
    rename(&mut body, &fresh);
    each_expr_mut(&mut body, &mut program.vars, &mut |expr| {
        if let ExprKind::Var(var) = &mut expr.kind
            && let Some(&new) = fresh.get(var)
        {
            *var = new;
        }
    });
    Function { body, ..copy }
}

/// Calls `found` on each variable that the statements declare.
fn declared(stmts: &[Stmt], found: &mut impl FnMut(VarId)) {
    for stmt in stmts {
        match stmt {
            Stmt::Decl(var) => found(*var),
            Stmt::IfSome {
                var, then, other, ..
            } => {
                found(*var);
                declared(then, found);
                declared(other, found);
            }
            Stmt::LetSome { var, otherwise, .. } => {
                found(*var);
                declared(otherwise, found);
            }
            Stmt::Parse {
                number,
                error,
                end,
                start,
                ..
            } => {
                for var in number.iter().chain(error).chain([end]) {
                    found(*var);
                }
                if let Some((_, var)) = start {
                    found(*var);
                }
            }
            Stmt::Block(body)
            | Stmt::While(_, body)
            | Stmt::DoWhile(body, _)
            | Stmt::Switch(_, body) => declared(body, found),
            Stmt::If(_, then, other) => {
                declared(then, found);
                declared(other, found);
            }
            Stmt::For { init, body, .. } => {
                declared(init, found);
                declared(body, found);
            }
            _ => {}
        }
    }
}

/// Gives the variables that the statements declare their new numbers.
fn rename(stmts: &mut [Stmt], fresh: &HashMap<VarId, VarId>) {
    let new = |var: &mut VarId| {
        if let Some(&renamed) = fresh.get(var) {
            *var = renamed;
        }
    };
    for stmt in stmts {
        match stmt {
            Stmt::Decl(var) => new(var),
            Stmt::IfSome {
                var, then, other, ..
            } => {
                new(var);
                rename(then, fresh);
                rename(other, fresh);
            }
            Stmt::LetSome { var, otherwise, .. } => {
                new(var);
                rename(otherwise, fresh);
            }
            Stmt::Parse {
                number,
                error,
                end,
                start,
                ..
            } => {
                for var in number.iter_mut().chain(error).chain([end]) {
                    new(var);
                }
                if let Some((_, var)) = start {
                    new(var);
                }
            }
            Stmt::Block(body)
            | Stmt::While(_, body)
            | Stmt::DoWhile(body, _)
            | Stmt::Switch(_, body) => rename(body, fresh),
            Stmt::If(_, then, other) => {
                rename(then, fresh);
                rename(other, fresh);
            }
            Stmt::For { init, body, .. } => {
                rename(init, fresh);
                rename(body, fresh);
            }
            _ => {}
        }
    }
}
