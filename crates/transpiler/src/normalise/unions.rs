//! `unions`, R33: a union whose members are all numbers of one size is the
//! bits of a number, which the program reads as one type or another. Safe
//! Rust has no memory that two types share, and it has `to_bits` and
//! `from_bits`. So the union becomes the unsigned integer of its size, a
//! store into a member converts the value to those bits, and a read
//! converts them back. After this pass the tree has no such union, and
//! every later stage sees an integer.

use crate::Unsupported;
use crate::c::{Expr, ExprKind, Program, Stmt, Type};

type R<T> = Result<T, Unsupported>;

pub fn unions(program: &mut Program) -> R<()> {
    let views: Vec<Option<View>> = (0..program.records.len())
        .map(|id| {
            program.bits_of_union(id).map(|bits| View {
                bits: Type::Int {
                    bits,
                    signed: false,
                },
                members: program.records[id]
                    .fields
                    .iter()
                    .map(|field| field.ty.clone())
                    .collect(),
            })
        })
        .collect();
    if views.iter().all(Option::is_none) {
        return Ok(());
    }
    let unions = Unions { views };
    for var in &mut program.vars {
        unions.retype(&mut var.ty);
        for part in var.init.iter_mut().chain(&mut var.len) {
            unions.value(part)?;
        }
    }
    for record in &mut program.records {
        for field in &mut record.fields {
            unions.retype(&mut field.ty);
        }
    }
    for function in &mut program.functions {
        unions.retype(&mut function.ret);
        unions.block(&mut function.body)?;
    }
    Ok(())
}

/// A union that is the bits of a number.
struct View {
    /// The unsigned integer of the union's size.
    bits: Type,
    /// The types of its members.
    members: Vec<Type>,
}

struct Unions {
    /// For each record, how it is read if it is such a union.
    views: Vec<Option<View>>,
}

impl Unions {
    /// Writes the integer for the union in a type.
    fn retype(&self, ty: &mut Type) {
        match ty {
            Type::Record(id) => {
                if let Some(view) = &self.views[*id] {
                    *ty = view.bits.clone();
                }
            }
            Type::Pointer(to) | Type::Array(to, _) => self.retype(to),
            Type::Function { ret, params, .. } => {
                self.retype(ret);
                params.iter_mut().for_each(|param| self.retype(param));
            }
            _ => {}
        }
    }

    /// The union and the type of the member, if the place is a member of
    /// such a union that is not the bits themselves.
    fn member(&self, place: &Expr) -> Option<(&View, &Type)> {
        let ExprKind::Member { record, field, .. } = &place.kind else {
            return None;
        };
        let view = self.views[*record].as_ref()?;
        let member = &view.members[*field];
        (*member != view.bits).then_some((view, member))
    }

    fn block(&self, stmts: &mut [Stmt]) -> R<()> {
        for stmt in stmts {
            match stmt {
                Stmt::Expr(expr) => self.effect(expr)?,
                Stmt::Return(Some(expr)) => self.value(expr)?,
                Stmt::Block(body) => self.block(body)?,
                Stmt::If(cond, then, other) => {
                    self.value(cond)?;
                    self.block(then)?;
                    self.block(other)?;
                }
                Stmt::While(cond, body) | Stmt::DoWhile(body, cond) | Stmt::Switch(cond, body) => {
                    self.value(cond)?;
                    self.block(body)?;
                }
                Stmt::For {
                    init,
                    cond,
                    step,
                    body,
                } => {
                    self.block(init)?;
                    if let Some(cond) = cond {
                        self.value(cond)?;
                    }
                    if let Some(step) = step {
                        self.effect(step)?;
                    }
                    self.block(body)?;
                }
                Stmt::IfSome {
                    source,
                    then,
                    other,
                    ..
                } => {
                    self.value(source)?;
                    self.block(then)?;
                    self.block(other)?;
                }
                Stmt::LetSome {
                    source, otherwise, ..
                } => {
                    self.value(source)?;
                    self.block(otherwise)?;
                }
                Stmt::Parse { call, start, .. } => {
                    self.value(call)?;
                    if let Some((start, _)) = start {
                        self.value(start)?;
                    }
                }
                // The initialiser of a declared variable is rewritten
                // with the variable.
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

    /// An expression whose value nothing uses. Here a store into a member
    /// is a store of the bits; where its value is used, that value has the
    /// member's type, for which there is no rule yet.
    fn effect(&self, expr: &mut Expr) -> R<()> {
        match &mut expr.kind {
            ExprKind::Comma(left, right) => {
                self.effect(left)?;
                self.effect(right)?;
                expr.ty = right.ty.clone();
                Ok(())
            }
            ExprKind::Assign {
                op: None,
                target,
                value,
            } if let Some((view, member)) = self.member(target) => {
                self.value(value)?;
                if value.ty != *member {
                    let at = value.at;
                    let written = std::mem::replace(&mut **value, hole());
                    **value = Expr {
                        ty: member.clone(),
                        kind: ExprKind::Cast(Box::new(written)),
                        at,
                    };
                }
                let at = value.at;
                let written = std::mem::replace(&mut **value, hole());
                **value = Expr {
                    ty: view.bits.clone(),
                    kind: match member {
                        Type::Float { .. } => ExprKind::Bits(Box::new(written)),
                        _ => ExprKind::Cast(Box::new(written)),
                    },
                    at,
                };
                expr.ty = view.bits.clone();
                // What is left of the target is the union itself.
                self.value(target)?;
                let (ExprKind::Cast(bits) | ExprKind::Bits(bits)) = &mut target.kind else {
                    unreachable!("a member of a union that is read as its bits");
                };
                **target = std::mem::replace(&mut **bits, hole());
                Ok(())
            }
            _ => self.value(expr),
        }
    }

    fn value(&self, expr: &mut Expr) -> R<()> {
        self.retype(&mut expr.ty);
        let at = expr.at;
        match &mut expr.kind {
            ExprKind::SizeOf(ty) => self.retype(ty),
            ExprKind::Assign { op, target, .. } if self.member(target).is_some() => {
                return Err(Unsupported::new(
                    match op {
                        Some(_) => "an update in place of a member of a union: no rule yet",
                        None => "a store into a member of a union whose value is used: no rule yet",
                    },
                    at,
                ));
            }
            ExprKind::IncDec { target, .. } if self.member(target).is_some() => {
                return Err(Unsupported::new(
                    "an update in place of a member of a union: no rule yet",
                    at,
                ));
            }
            ExprKind::AddrOf(target) if self.member(target).is_some() => {
                return Err(Unsupported::new(
                    "the address of a member of a union: no rule yet",
                    at,
                ));
            }
            _ => {}
        }
        for child in expr.children_mut() {
            self.value(child)?;
        }
        let ExprKind::Member {
            base,
            record,
            field,
            arrow,
        } = &mut expr.kind
        else {
            return Ok(());
        };
        let Some(view) = &self.views[*record] else {
            return Ok(());
        };
        let member = &view.members[*field];
        let mut bits = std::mem::replace(&mut **base, hole());
        if *arrow {
            bits = Expr {
                ty: view.bits.clone(),
                kind: ExprKind::Deref(Box::new(bits)),
                at,
            };
        }
        *expr = match member {
            member if *member == view.bits => bits,
            Type::Float { .. } => Expr {
                ty: member.clone(),
                kind: ExprKind::Bits(Box::new(bits)),
                at,
            },
            // Between integers of one size, C's conversion keeps the bits.
            member => Expr {
                ty: member.clone(),
                kind: ExprKind::Cast(Box::new(bits)),
                at,
            },
        };
        Ok(())
    }
}

/// What stands where an expression has been taken out.
fn hole() -> Expr {
    Expr {
        ty: Type::Void,
        kind: ExprKind::Null,
        at: 0,
    }
}
