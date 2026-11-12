//! The translation rules. Each turns one C construct into Rust syntax,
//! reading the facts for whatever depends on the rest of the program.
//! `CATALOGUE.md` lists the rules; what has none is `Unsupported`.

mod expr;
mod library;
mod pointer;
mod stmt;
mod types;

use crate::Unsupported;
use crate::c::{Expr, ExprKind, FuncId, Program, Stmt, Type, VarId, any_expr};
use crate::facts::{Access, Facts, Slot};
use crate::names::{Names, plain};
use crate::rust;
use std::collections::BTreeSet;
use syn::parse_quote;

type R<T> = Result<T, Unsupported>;

fn unsupported<T>(what: impl Into<String>, at: u32) -> R<T> {
    Err(Unsupported::new(what, at))
}

/// A function that the translation adds once, when it calls it: one C
/// library function's behaviour on Rust types, where the standard library
/// has no function that behaves the same on every input.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Helper {
    ParseInt,
}

impl Helper {
    fn item(self) -> syn::Item {
        match self {
            Helper::ParseInt => parse_quote! {
                /// The number at the start of `text` as C's `strtoll` reads it in
                /// base 10: white space, a sign, then digits. It is 0 without
                /// digits and the nearest limit when out of range.
                fn parse_int(text: &str) -> i64 {
                    let text = text.trim_start_matches(|c| c == ' ' || ('\t'..='\r').contains(&c));
                    let (negative, digits) = match text.strip_prefix('-') {
                        Some(digits) => (true, digits),
                        None => (false, text.strip_prefix('+').unwrap_or(text)),
                    };
                    let mut value: i64 = 0;
                    for digit in digits.bytes().take_while(u8::is_ascii_digit) {
                        let digit = i64::from(digit - b'0');
                        value = value.saturating_mul(10);
                        value = if negative {
                            value.saturating_sub(digit)
                        } else {
                            value.saturating_add(digit)
                        };
                    }
                    value
                }
            },
        }
    }
}

/// What `break` leaves.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Nest {
    Loop,
    Switch,
}

/// The item `struct Globals`, and the value `main` starts it with.
type State = (syn::Item, syn::Expr);

struct Lower<'a> {
    program: &'a Program,
    facts: &'a Facts,
    names: Names,
    helpers: BTreeSet<Helper>,
    /// The enumerators the translation names.
    enumerators: BTreeSet<usize>,
    /// The function being translated, or none for a global's initialiser.
    function: Option<FuncId>,
    /// `main`'s parameters while `main` is translated.
    argc: Option<VarId>,
    argv: Option<VarId>,
    /// Whether the function being translated is a `main` that returns an
    /// exit code.
    exits: bool,
    nesting: Vec<Nest>,
}

/// Translates the program.
pub fn program(program: &Program, facts: &Facts) -> R<syn::File> {
    let mut lower = Lower {
        program,
        facts,
        names: Names::new(program, facts),
        helpers: BTreeSet::new(),
        enumerators: BTreeSet::new(),
        function: None,
        argc: None,
        argv: None,
        exits: false,
        nesting: Vec::new(),
    };
    let mut records = Vec::new();
    for (id, record) in program.records.iter().enumerate() {
        if record.own {
            records.extend(lower.record(id)?);
        }
    }
    let (constants, state) = lower.globals()?;
    let mut functions = Vec::new();
    let mut uses_exit_code = false;
    for (id, function) in program.functions.iter().enumerate() {
        functions.push(if function.name == "main" {
            let main = lower.main(id, state.as_ref().map(|(_, init)| init))?;
            uses_exit_code = lower.exits;
            main
        } else {
            lower.function(id)?
        });
    }
    if program.function("main").is_none() {
        return unsupported("a program without `main`", 0);
    }
    let mut items: Vec<syn::Item> = Vec::new();
    if uses_exit_code {
        items.push(parse_quote!(
            use std::process::ExitCode;
        ));
    }
    for &index in &lower.enumerators {
        let enumerator = &program.enumerators[index];
        let (name, value) = (
            rust::ident(&enumerator.name),
            rust::int(enumerator.value, ""),
        );
        items.push(parse_quote!(const #name: i32 = #value;));
    }
    items.extend(constants);
    items.extend(records);
    items.extend(state.map(|(item, _)| item));
    items.extend(functions);
    items.extend(lower.helpers.iter().map(|helper| helper.item()));
    Ok(syn::File {
        shebang: None,
        attrs: Vec::new(),
        items,
    })
}

impl Lower<'_> {
    /// R5: a `struct` for each record, with the traits its uses need.
    fn record(&mut self, id: usize) -> R<Vec<syn::Item>> {
        let record = &self.program.records[id];
        if record.union {
            return unsupported(format!("the union `{}`", record.name), 0);
        }
        let name = rust::ident(self.names.record(id));
        let mut fields: Vec<syn::Field> = Vec::new();
        let mut zeros: Vec<syn::FieldValue> = Vec::new();
        for (index, field) in record.fields.iter().enumerate() {
            if field.bits.is_some() {
                return unsupported(format!("the bit-field `{}`", field.name), 0);
            }
            let slot = Slot::Field(id, index);
            let ty = self.field_type(slot, &field.ty)?;
            let zero = self.zero_of(&field.ty, Some(slot), 0)?;
            let field = rust::ident(&plain(&field.name));
            fields.push(
                syn::parse::Parser::parse2(syn::Field::parse_named, quote::quote!(#field: #ty))
                    .expect("a field"),
            );
            zeros.push(parse_quote!(#field: #zero));
        }
        let ty = Type::Record(id);
        let copy = self.is_copy(&ty).then(|| quote::quote!(Copy,));
        let derived = self.derives_default(&ty);
        let default = derived.then(|| quote::quote!(Default,));
        let mut items: Vec<syn::Item> = vec![parse_quote! {
            #[derive(Clone, #copy #default)]
            struct #name { #(#fields),* }
        }];
        if !derived {
            items.push(parse_quote! {
                impl Default for #name {
                    fn default() -> Self {
                        Self { #(#zeros),* }
                    }
                }
            });
        }
        Ok(items)
    }

    /// The type of a record's field or of a global, which holds its value
    /// or owns what it points to.
    fn field_type(&mut self, slot: Slot, ty: &Type) -> R<syn::Type> {
        if !ty.is_pointer() {
            return self.ty(ty, 0);
        }
        if !self.repr(slot, 0)?.owned {
            return unsupported(
                format!("{} borrows what it points to", self.slot_name(slot)),
                0,
            );
        }
        self.slot_type(slot, ty, None, 0)
    }

    /// R9: a global that never changes is a constant; the others are the
    /// fields of `Globals`, which `main` creates and the functions that use
    /// them take as their first parameter. Returns the constants, and
    /// `Globals` with its first value.
    fn globals(&mut self) -> R<(Vec<syn::Item>, Option<State>)> {
        let program = self.program;
        let mut constants: Vec<syn::Item> = Vec::new();
        let mut fields: Vec<syn::Field> = Vec::new();
        let mut values: Vec<syn::FieldValue> = Vec::new();
        for &global in &program.globals {
            let var = &program.vars[global];
            let name = rust::ident(self.names.var(global));
            let slot = var.ty.is_pointer().then_some(Slot::Var(global));
            let value = match &var.init {
                Some(init) => self.init(&var.ty, slot, init)?,
                None => self.zero_of(&var.ty, slot, var.at)?,
            };
            if self.facts.mutated(global) {
                let ty = self.field_type(Slot::Var(global), &var.ty)?;
                fields.push(
                    syn::parse::Parser::parse2(syn::Field::parse_named, quote::quote!(#name: #ty))
                        .expect("a field"),
                );
                values.push(parse_quote!(#name: #value));
            } else if var.ty.is_pointer() {
                return unsupported(
                    format!("the global pointer `{}` that never changes", var.name),
                    var.at,
                );
            } else {
                let ty = self.ty(&var.ty, var.at)?;
                constants.push(match var.ty {
                    Type::Array(..) | Type::Record(_) => parse_quote!(static #name: #ty = #value;),
                    _ => parse_quote!(const #name: #ty = #value;),
                });
            }
        }
        if fields.is_empty() {
            return Ok((constants, None));
        }
        let item = parse_quote! {
            /// The program's global variables.
            struct Globals { #(#fields),* }
        };
        Ok((
            constants,
            Some((item, parse_quote!(Globals { #(#values),* }))),
        ))
    }

    /// Whether a local or a parameter needs `mut`.
    fn needs_mut(&self, var: VarId) -> bool {
        let pointer = self.facts.pointer(Slot::Var(var));
        // An owner and an `Option` are written through their binding.
        self.facts.mutated(var)
            || (self.facts.stored_through(var) && (pointer.owned || pointer.nullable))
    }

    /// R1: a function, with a borrow for each pointer it does not own.
    fn function(&mut self, id: FuncId) -> R<syn::Item> {
        let program = self.program;
        let function = &program.functions[id];
        if function.variadic {
            return unsupported(format!("the variadic function `{}`", function.name), 0);
        }
        self.function = Some(id);
        self.exits = false;
        let access = self.facts.access(id);
        let borrows = |lower: &Self, slot: Slot| lower.repr(slot, 0).map(|repr| !repr.owned);
        let mut borrowed = usize::from(access != Access::None);
        for &param in &function.params {
            if program.vars[param].ty.is_pointer() && borrows(self, Slot::Var(param))? {
                borrowed += 1;
            }
        }
        // A borrowed result with one borrowed parameter needs no lifetime.
        // With none it can only point to a constant, and with several the
        // translation does not know which, so all share one.
        let returns_borrow = function.ret.is_pointer() && borrows(self, Slot::Return(id))?;
        let (shared, result): (Option<syn::Lifetime>, Option<syn::Lifetime>) =
            match (returns_borrow, borrowed) {
                (false, _) | (true, 1) => (None, None),
                (true, 0) => (None, Some(parse_quote!('static))),
                (true, _) => (Some(parse_quote!('a)), Some(parse_quote!('a))),
            };
        let mut params: Vec<syn::FnArg> = Vec::new();
        match access {
            Access::None => {}
            Access::Read => params.push(parse_quote!(globals: & #shared Globals)),
            Access::Write => params.push(parse_quote!(globals: & #shared mut Globals)),
        }
        for &param in &function.params {
            let var = &program.vars[param];
            let name = rust::ident(self.names.var(param));
            let mutability: Option<syn::Token![mut]> = self.needs_mut(param).then(Default::default);
            let ty = match var.ty.is_pointer() {
                true => self.slot_type(Slot::Var(param), &var.ty, shared.as_ref(), var.at)?,
                false => self.ty(&var.ty, var.at)?,
            };
            params.push(parse_quote!(#mutability #name: #ty));
        }
        let ret = match &function.ret {
            Type::Void => None,
            ty if ty.is_pointer() => {
                Some(self.slot_type(Slot::Return(id), ty, result.as_ref(), 0)?)
            }
            ty => Some(self.ty(ty, 0)?),
        };
        let body = self.body(&function.body)?;
        let name = rust::ident(self.names.function(id));
        let (generics, ret) = (shared.iter(), ret.iter());
        Ok(parse_quote! {
            fn #name #(<#generics>)* (#(#params),*) #(-> #ret)* {
                #(#body)*
            }
        })
    }

    /// R1: `main`, which returns an exit code unless it only ever returns
    /// 0, and reads its arguments from `args`.
    fn main(&mut self, id: FuncId, state: Option<&syn::Expr>) -> R<syn::Item> {
        let program = self.program;
        let function = &program.functions[id];
        self.function = Some(id);
        (self.argc, self.argv) = match function.params[..] {
            [] => (None, None),
            [argc, argv] => (Some(argc), Some(argv)),
            _ => return unsupported("a `main` with other parameters", 0),
        };
        let mut body: Vec<syn::Stmt> = Vec::new();
        let named = |var: Option<VarId>| {
            any_expr(
                &function.body,
                &program.vars,
                &mut |expr| matches!(expr.kind, ExprKind::Var(named) if Some(named) == var),
            )
        };
        if named(self.argc) || named(self.argv) {
            body.push(parse_quote!(let args: Vec<String> = std::env::args().collect();));
        }
        if let Some(state) = state {
            body.push(match self.facts.access(id) {
                Access::None => parse_quote!(let _globals = #state;),
                Access::Read => parse_quote!(let globals = &#state;),
                Access::Write => parse_quote!(let globals = &mut #state;),
            });
        }
        self.exits = !returns_only_zero(&function.body, program, self.facts);
        body.extend(self.body(&function.body)?);
        if self.exits && !matches!(function.body.last(), Some(Stmt::Return(Some(_)))) {
            body.push(syn::Stmt::Expr(parse_quote!(ExitCode::SUCCESS), None));
        }
        (self.argc, self.argv) = (None, None);
        let ret = self.exits.then(|| quote::quote!(-> ExitCode));
        Ok(parse_quote! {
            fn main() #ret {
                #(#body)*
            }
        })
    }

    /// The statements of a function, whose last `return` is its value.
    fn body(&mut self, stmts: &[Stmt]) -> R<Vec<syn::Stmt>> {
        let (rest, last) = match stmts.split_last() {
            Some((Stmt::Return(Some(value)), rest)) => (rest, Some(value)),
            _ => (stmts, None),
        };
        let mut body = self.stmts(rest)?;
        if let Some(value) = last {
            if !self.in_main() {
                body.push(syn::Stmt::Expr(self.returned(value)?, None));
            } else if self.exits {
                body.push(syn::Stmt::Expr(self.exit_code(value)?, None));
            }
            // A `main` without an exit code ends with `return 0`, which
            // needs no code.
        }
        Ok(body)
    }

    /// Whether the function being translated is `main`.
    fn in_main(&self) -> bool {
        self.function
            .is_some_and(|id| self.program.functions[id].name == "main")
    }

    /// The value a function returns for `return value`.
    fn returned(&mut self, value: &Expr) -> R<syn::Expr> {
        match (self.function, value.ty.is_pointer()) {
            (Some(function), true) => {
                let repr = self.repr(Slot::Return(function), value.at)?;
                self.pointer(value, repr)
            }
            _ => self.typed(value),
        }
    }

    /// The exit code of `main` for `return value`.
    fn exit_code(&mut self, value: &Expr) -> R<syn::Expr> {
        Ok(match value.constant(self.program) {
            Some(0) => parse_quote!(ExitCode::SUCCESS),
            // The system keeps the low byte.
            Some(code) => rust::call("ExitCode::from", [rust::int(code & 0xff, "")]),
            None => {
                let code = rust::cast(self.expr(value)?, parse_quote!(u8));
                rust::call("ExitCode::from", [code])
            }
        })
    }
}

/// Whether every `return` that `main` reaches returns the constant 0.
fn returns_only_zero(stmts: &[Stmt], program: &Program, facts: &Facts) -> bool {
    let only_zero = |stmts: &[Stmt]| returns_only_zero(stmts, program, facts);
    stmts.iter().all(|stmt| match stmt {
        Stmt::Return(Some(value)) => value.constant(program) == Some(0),
        Stmt::Return(None) => false,
        Stmt::Block(body)
        | Stmt::While(_, body)
        | Stmt::DoWhile(body, _)
        | Stmt::Switch(_, body)
        | Stmt::For { body, .. } => only_zero(body),
        // A branch that the facts rule out returns nothing.
        Stmt::If(cond, then, other) => match facts.truth(program, cond) {
            Some(true) => only_zero(then),
            Some(false) => only_zero(other),
            None => only_zero(then) && only_zero(other),
        },
        Stmt::IfSome { then, other, .. } => only_zero(then) && only_zero(other),
        Stmt::LetSome { otherwise, .. } => only_zero(otherwise),
        _ => true,
    })
}
