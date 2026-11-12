//! The C program as the rules see it: Clang's syntax tree with every type
//! resolved and every declaration numbered. `clang` builds it, `normalise`
//! rewrites it, and no later stage reads JSON.

pub type VarId = usize;
pub type RecordId = usize;
pub type FuncId = usize;

#[derive(Debug, Default)]
pub struct Program {
    /// The records of the program's own file, and those of headers without
    /// their fields.
    pub records: Vec<Record>,
    pub enumerators: Vec<Enumerator>,
    /// Every variable of the program's own file: globals, parameters and
    /// locals.
    pub vars: Vec<Var>,
    /// The variables that live as long as the program, in source order:
    /// those at file scope and the `static` ones of functions.
    pub globals: Vec<VarId>,
    /// The program's own functions that have a body, in source order.
    pub functions: Vec<Function>,
}

impl Program {
    /// The program's own function of this name, if it has a body.
    pub fn function(&self, name: &str) -> Option<FuncId> {
        self.functions.iter().position(|f| f.name == name)
    }
}

#[derive(Debug)]
pub struct Record {
    pub name: String,
    pub union: bool,
    /// Empty for a record from a header.
    pub fields: Vec<Field>,
    /// Whether the program's own file defines it.
    pub own: bool,
}

#[derive(Debug)]
pub struct Field {
    pub name: String,
    pub ty: Type,
    /// The width of a bit-field.
    pub bits: Option<u32>,
}

#[derive(Debug)]
pub struct Enumerator {
    pub name: String,
    pub value: i128,
}

#[derive(Clone, Debug)]
pub struct Var {
    pub name: String,
    pub ty: Type,
    pub init: Option<Expr>,
    pub kind: VarKind,
    pub at: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VarKind {
    /// At file scope.
    Global,
    /// `static` in the function.
    Static(FuncId),
    Local,
    Param,
}

impl VarKind {
    /// Whether the variable lives as long as the program.
    pub fn is_global(self) -> bool {
        matches!(self, VarKind::Global | VarKind::Static(_))
    }
}

#[derive(Debug)]
pub struct Function {
    pub name: String,
    pub ret: Type,
    pub params: Vec<VarId>,
    pub variadic: bool,
    pub body: Vec<Stmt>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Type {
    Void,
    Bool,
    /// Plain `char`: text. It is signed in C on x86-64, which matters when
    /// it is converted to a wider type.
    Char,
    Int {
        bits: u8,
        signed: bool,
    },
    Float {
        bits: u8,
    },
    Pointer(Box<Type>),
    /// The length is `None` when it is not a constant, as in a
    /// variable-length array.
    Array(Box<Type>, Option<u64>),
    Record(RecordId),
    Function {
        ret: Box<Type>,
        params: Vec<Type>,
        variadic: bool,
    },
}

pub const INT: Type = Type::Int {
    bits: 32,
    signed: true,
};

impl Type {
    pub fn is_pointer(&self) -> bool {
        matches!(self, Type::Pointer(_))
    }

    pub fn is_integer(&self) -> bool {
        matches!(self, Type::Bool | Type::Char | Type::Int { .. })
    }

    /// The type a pointer points to or an array holds.
    pub fn element(&self) -> Option<&Type> {
        match self {
            Type::Pointer(to) | Type::Array(to, _) => Some(to),
            _ => None,
        }
    }

    /// Bits and signedness of an integer type as C computes with it.
    pub fn int(&self) -> Option<(u8, bool)> {
        match *self {
            Type::Bool => Some((1, false)),
            Type::Char => Some((8, true)),
            Type::Int { bits, signed } => Some((bits, signed)),
            _ => None,
        }
    }

    /// Size and alignment in bytes on x86-64, for `sizeof`.
    pub fn layout(&self, program: &Program) -> Option<(u64, u64)> {
        Some(match self {
            Type::Void | Type::Function { .. } => return None,
            Type::Bool | Type::Char => (1, 1),
            Type::Int { bits, .. } | Type::Float { bits } => {
                (u64::from(*bits) / 8, u64::from(*bits) / 8)
            }
            Type::Pointer(_) => (8, 8),
            Type::Array(of, len) => {
                let (size, align) = of.layout(program)?;
                (size * (*len)?, align)
            }
            Type::Record(id) => {
                let record = &program.records[*id];
                if !record.own || record.fields.iter().any(|field| field.bits.is_some()) {
                    return None;
                }
                let (mut size, mut align) = (0, 1);
                for field in &record.fields {
                    let (field_size, field_align) = field.ty.layout(program)?;
                    align = align.max(field_align);
                    if record.union {
                        size = size.max(field_size);
                    } else {
                        size = size.next_multiple_of(field_align) + field_size;
                    }
                }
                (size.next_multiple_of(align), align)
            }
        })
    }
}

#[derive(Clone, Debug)]
pub struct Expr {
    pub ty: Type,
    pub kind: ExprKind,
    /// The offset in the source file, for messages; 0 when unknown.
    pub at: u32,
}

#[derive(Clone, Debug)]
pub enum ExprKind {
    Int(i128),
    /// A character literal such as `'a'`; its type is `int`.
    Char(u8),
    Float(f64),
    /// A string literal without its final NUL. The type is the array it
    /// fills, which may be longer.
    Str(Vec<u8>),
    Var(VarId),
    Enumerator(usize),
    /// A function by name: the program's own or the library's.
    Function(String),
    /// A variable that a header declares, such as `stdin`.
    Extern(String),
    /// A null pointer constant.
    Null,
    /// The zero value of the type, for what an initialiser leaves out.
    Zero,
    Unary(UnOp, Box<Expr>),
    IncDec {
        target: Box<Expr>,
        increment: bool,
        prefix: bool,
    },
    AddrOf(Box<Expr>),
    Deref(Box<Expr>),
    /// An array used as a pointer to its first element.
    Decay(Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    /// `target = value`, or `target op= value` computed in the type
    /// `compute`.
    Assign {
        op: Option<(BinOp, Type)>,
        target: Box<Expr>,
        value: Box<Expr>,
    },
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
    Comma(Box<Expr>, Box<Expr>),
    Call(Box<Expr>, Vec<Expr>),
    Member {
        base: Box<Expr>,
        record: RecordId,
        field: usize,
        arrow: bool,
    },
    /// `base[index]`, where `base` is a pointer.
    Index(Box<Expr>, Box<Expr>),
    /// A conversion to the expression's type, written or implied.
    Cast(Box<Expr>),
    SizeOf(Type),
    /// The values of an array's elements or a record's fields in order.
    /// Elements beyond them are zero.
    InitList(Vec<Expr>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    /// `~`
    BitNot,
    /// `!`
    Not,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Shl,
    Shr,
    BitAnd,
    BitOr,
    BitXor,
    Lt,
    Gt,
    Le,
    Ge,
    Eq,
    Ne,
    /// `&&`
    And,
    /// `||`
    Or,
}

impl BinOp {
    pub fn is_comparison(self) -> bool {
        use BinOp::*;
        matches!(self, Lt | Gt | Le | Ge | Eq | Ne)
    }

    pub fn is_logical(self) -> bool {
        matches!(self, BinOp::And | BinOp::Or)
    }
}

#[derive(Debug)]
pub enum Stmt {
    Expr(Expr),
    /// The declaration of a local variable; its initialiser is in `Var`.
    Decl(VarId),
    Block(Vec<Stmt>),
    If(Expr, Vec<Stmt>, Vec<Stmt>),
    While(Expr, Vec<Stmt>),
    DoWhile(Vec<Stmt>, Expr),
    For {
        init: Vec<Stmt>,
        cond: Option<Expr>,
        step: Option<Expr>,
        body: Vec<Stmt>,
    },
    /// The body holds `Case` and `Default` where C has the labels.
    Switch(Expr, Vec<Stmt>),
    Case(i128),
    Default,
    Break,
    Continue,
    Return(Option<Expr>),
    Goto(String),
    Label(String),
    /// `normalise` writes these two for tests of a pointer against NULL.
    /// `var` is a new variable that holds `source` where it is not NULL.
    /// `if (p) then else other`, with `var` for `p` in `then`:
    IfSome {
        var: VarId,
        source: Expr,
        then: Vec<Stmt>,
        other: Vec<Stmt>,
    },
    /// `if (!p) otherwise`, where `otherwise` leaves the block, with `var`
    /// for `p` in the statements after it:
    LetSome {
        var: VarId,
        source: Expr,
        otherwise: Vec<Stmt>,
    },
}

impl Expr {
    /// The expression without the conversions around it.
    pub fn uncast(&self) -> &Expr {
        match &self.kind {
            ExprKind::Cast(inner) => inner.uncast(),
            _ => self,
        }
    }

    /// The value of a constant integer expression, with `sizeof` as on
    /// x86-64.
    pub fn constant(&self, program: &Program) -> Option<i128> {
        let value = match &self.kind {
            ExprKind::Int(value) => *value,
            ExprKind::Char(value) => i128::from(*value),
            ExprKind::Enumerator(index) => program.enumerators[*index].value,
            ExprKind::SizeOf(ty) => i128::from(ty.layout(program)?.0),
            ExprKind::Cast(inner) => inner.constant(program)?,
            ExprKind::Unary(op, inner) => {
                let value = inner.constant(program)?;
                match op {
                    UnOp::Neg => value.checked_neg()?,
                    UnOp::BitNot => !value,
                    UnOp::Not => i128::from(value == 0),
                }
            }
            ExprKind::Binary(op, left, right) => {
                let (left, right) = (left.constant(program)?, right.constant(program)?);
                match op {
                    BinOp::Add => left.checked_add(right)?,
                    BinOp::Sub => left.checked_sub(right)?,
                    BinOp::Mul => left.checked_mul(right)?,
                    BinOp::Div => left.checked_div(right)?,
                    BinOp::Rem => left.checked_rem(right)?,
                    BinOp::Shl => left.checked_shl(u32::try_from(right).ok()?)?,
                    BinOp::Shr => left.checked_shr(u32::try_from(right).ok()?)?,
                    BinOp::BitAnd => left & right,
                    BinOp::BitOr => left | right,
                    BinOp::BitXor => left ^ right,
                    BinOp::Lt => i128::from(left < right),
                    BinOp::Gt => i128::from(left > right),
                    BinOp::Le => i128::from(left <= right),
                    BinOp::Ge => i128::from(left >= right),
                    BinOp::Eq => i128::from(left == right),
                    BinOp::Ne => i128::from(left != right),
                    BinOp::And => i128::from(left != 0 && right != 0),
                    BinOp::Or => i128::from(left != 0 || right != 0),
                }
            }
            _ => return None,
        };
        // The value as the expression's type holds it.
        Some(match self.ty.int() {
            Some((1, _)) => i128::from(value != 0),
            Some((bits, true)) => {
                let shift = 128 - u32::from(bits);
                (value << shift) >> shift
            }
            Some((bits, false)) => value & ((1 << bits) - 1),
            None => return None,
        })
    }
}

impl Expr {
    /// The expressions directly inside this one, in evaluation order.
    pub fn children(&self) -> Vec<&Expr> {
        match &self.kind {
            ExprKind::Int(_)
            | ExprKind::Char(_)
            | ExprKind::Float(_)
            | ExprKind::Str(_)
            | ExprKind::Var(_)
            | ExprKind::Enumerator(_)
            | ExprKind::Function(_)
            | ExprKind::Extern(_)
            | ExprKind::Null
            | ExprKind::Zero
            | ExprKind::SizeOf(_) => Vec::new(),
            ExprKind::Unary(_, inner)
            | ExprKind::IncDec { target: inner, .. }
            | ExprKind::AddrOf(inner)
            | ExprKind::Deref(inner)
            | ExprKind::Decay(inner)
            | ExprKind::Cast(inner)
            | ExprKind::Member { base: inner, .. } => vec![inner],
            ExprKind::Binary(_, left, right)
            | ExprKind::Comma(left, right)
            | ExprKind::Index(left, right)
            | ExprKind::Assign {
                target: left,
                value: right,
                ..
            } => vec![left, right],
            ExprKind::Cond(cond, then, other) => vec![cond, then, other],
            ExprKind::Call(callee, args) => std::iter::once(&**callee).chain(args).collect(),
            ExprKind::InitList(items) => items.iter().collect(),
        }
    }

    /// The name of the function this expression names, if it does.
    pub fn function(&self) -> Option<&str> {
        match &self.kind {
            ExprKind::Function(name) => Some(name),
            _ => None,
        }
    }
}

impl Expr {
    /// Whether the test holds for this expression or one inside it.
    pub fn any(&self, test: &mut impl FnMut(&Expr) -> bool) -> bool {
        test(self) || self.children().into_iter().any(|child| child.any(test))
    }

    /// Whether evaluating the expression changes nothing: it has no
    /// assignment and no call.
    pub fn is_pure(&self) -> bool {
        !self.any(&mut |expr| {
            matches!(
                expr.kind,
                ExprKind::Assign { .. } | ExprKind::IncDec { .. } | ExprKind::Call(..)
            )
        })
    }

    /// The variable that holds the place, if it is one of a variable's own
    /// parts, such as `v.field[2]`.
    pub fn root_var(&self) -> Option<VarId> {
        match &self.kind {
            ExprKind::Var(var) => Some(*var),
            ExprKind::Member {
                base, arrow: false, ..
            } => base.root_var(),
            ExprKind::Index(base, _) => match &base.kind {
                ExprKind::Decay(array) => array.root_var(),
                _ => None,
            },
            _ => None,
        }
    }
}

/// Whether the test holds for an expression in the statements, at any depth.
/// `vars` holds the initialisers of the variables they declare.
pub fn any_expr(stmts: &[Stmt], vars: &[Var], test: &mut impl FnMut(&Expr) -> bool) -> bool {
    stmts.iter().any(|stmt| match stmt {
        Stmt::Expr(expr) | Stmt::Return(Some(expr)) => expr.any(test),
        Stmt::Decl(var) => vars[*var].init.as_ref().is_some_and(|init| init.any(test)),
        Stmt::Block(body) => any_expr(body, vars, test),
        Stmt::If(cond, then, other) => {
            cond.any(test) || any_expr(then, vars, test) || any_expr(other, vars, test)
        }
        Stmt::While(cond, body) | Stmt::DoWhile(body, cond) | Stmt::Switch(cond, body) => {
            cond.any(test) || any_expr(body, vars, test)
        }
        Stmt::For {
            init,
            cond,
            step,
            body,
        } => {
            any_expr(init, vars, test)
                || cond.iter().chain(step).any(|part| part.any(test))
                || any_expr(body, vars, test)
        }
        Stmt::IfSome {
            source,
            then,
            other,
            ..
        } => source.any(test) || any_expr(then, vars, test) || any_expr(other, vars, test),
        Stmt::LetSome {
            source, otherwise, ..
        } => source.any(test) || any_expr(otherwise, vars, test),
        Stmt::Return(None)
        | Stmt::Case(_)
        | Stmt::Default
        | Stmt::Break
        | Stmt::Continue
        | Stmt::Goto(_)
        | Stmt::Label(_) => false,
    })
}

impl Expr {
    /// Calls `change` on this expression and on every one inside it.
    pub fn each_mut(&mut self, change: &mut impl FnMut(&mut Expr)) {
        change(self);
        match &mut self.kind {
            ExprKind::Int(_)
            | ExprKind::Char(_)
            | ExprKind::Float(_)
            | ExprKind::Str(_)
            | ExprKind::Var(_)
            | ExprKind::Enumerator(_)
            | ExprKind::Function(_)
            | ExprKind::Extern(_)
            | ExprKind::Null
            | ExprKind::Zero
            | ExprKind::SizeOf(_) => {}
            ExprKind::Unary(_, inner)
            | ExprKind::IncDec { target: inner, .. }
            | ExprKind::AddrOf(inner)
            | ExprKind::Deref(inner)
            | ExprKind::Decay(inner)
            | ExprKind::Cast(inner)
            | ExprKind::Member { base: inner, .. } => inner.each_mut(change),
            ExprKind::Binary(_, left, right)
            | ExprKind::Comma(left, right)
            | ExprKind::Index(left, right)
            | ExprKind::Assign {
                target: left,
                value: right,
                ..
            } => {
                left.each_mut(change);
                right.each_mut(change);
            }
            ExprKind::Cond(cond, then, other) => {
                for part in [cond, then, other] {
                    part.each_mut(change);
                }
            }
            ExprKind::Call(callee, args) => {
                callee.each_mut(change);
                for arg in args {
                    arg.each_mut(change);
                }
            }
            ExprKind::InitList(items) => {
                for item in items {
                    item.each_mut(change);
                }
            }
        }
    }
}

/// Calls `change` on every expression in the statements, at any depth,
/// and in the initialisers of the variables they declare.
pub fn each_expr_mut(stmts: &mut [Stmt], vars: &mut [Var], change: &mut impl FnMut(&mut Expr)) {
    for stmt in stmts {
        match stmt {
            Stmt::Expr(expr) | Stmt::Return(Some(expr)) => expr.each_mut(change),
            Stmt::Decl(var) => {
                if let Some(init) = &mut vars[*var].init {
                    init.each_mut(change);
                }
            }
            Stmt::Block(body) => each_expr_mut(body, vars, change),
            Stmt::If(cond, then, other) => {
                cond.each_mut(change);
                each_expr_mut(then, vars, change);
                each_expr_mut(other, vars, change);
            }
            Stmt::While(cond, body) | Stmt::DoWhile(body, cond) | Stmt::Switch(cond, body) => {
                cond.each_mut(change);
                each_expr_mut(body, vars, change);
            }
            Stmt::For {
                init,
                cond,
                step,
                body,
            } => {
                each_expr_mut(init, vars, change);
                for part in cond.iter_mut().chain(step) {
                    part.each_mut(change);
                }
                each_expr_mut(body, vars, change);
            }
            Stmt::IfSome {
                source,
                then,
                other,
                ..
            } => {
                source.each_mut(change);
                each_expr_mut(then, vars, change);
                each_expr_mut(other, vars, change);
            }
            Stmt::LetSome {
                source, otherwise, ..
            } => {
                source.each_mut(change);
                each_expr_mut(otherwise, vars, change);
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
}
