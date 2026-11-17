//! The analysis: what the whole program does with each pointer, variable and
//! global. The rules read these facts and never look beyond the expression
//! they translate, so a question about the rest of the program belongs here.
//!
//! A *slot* is a place that holds a pointer: a variable, a record's field or
//! a function's result. Every assignment, argument and `return` is a *flow*
//! from a pointer's source into a slot, and the facts spread along the flows
//! until nothing changes.

use crate::c::{
    BinOp, Expr, ExprKind, FuncId, Program, RecordId, Stmt, Type, UnOp, VarId, VarKind,
};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Slot {
    Var(VarId),
    Field(RecordId, usize),
    Return(FuncId),
    /// What a pointer to a pointer points to: the place that holds a
    /// pointer, whose facts are those of every place it may name.
    Behind(Outer),
}

/// A slot that holds a pointer to a pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Outer {
    Var(VarId),
    Field(RecordId, usize),
    Return(FuncId),
}

impl Slot {
    /// The slot behind this slot's pointers, one level down: a pointer
    /// behind a pointer behind a pointer has none.
    pub fn behind(self) -> Option<Slot> {
        Some(Slot::Behind(match self {
            Slot::Var(var) => Outer::Var(var),
            Slot::Field(record, field) => Outer::Field(record, field),
            Slot::Return(function) => Outer::Return(function),
            Slot::Behind(_) => return None,
        }))
    }

    /// The slot whose pointer points to this one, for a slot behind one.
    pub fn outer(self) -> Option<Slot> {
        match self {
            Slot::Behind(Outer::Var(var)) => Some(Slot::Var(var)),
            Slot::Behind(Outer::Field(record, field)) => Some(Slot::Field(record, field)),
            Slot::Behind(Outer::Return(function)) => Some(Slot::Return(function)),
            _ => None,
        }
    }
}

/// The C type of the pointers that a slot holds.
pub fn slot_type(program: &Program, slot: Slot) -> &Type {
    match slot {
        Slot::Var(var) => &program.vars[var].ty,
        Slot::Field(record, field) => &program.records[record].fields[field].ty,
        Slot::Return(function) => &program.functions[function].ret,
        Slot::Behind(_) => slot
            .outer()
            .and_then(|outer| slot_type(program, outer).element())
            .unwrap_or(&Type::Void),
    }
}

/// What the program does with the pointers a slot holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pointer {
    /// Something is stored through it, so a borrow must be `&mut`.
    pub written: bool,
    /// It points into an array: it is indexed or moved, or holds an array.
    pub many: bool,
    /// It holds memory from `malloc` that it must free or hand on.
    pub owned: bool,
    /// It may be NULL.
    pub nullable: bool,
    /// It may hold `argv[i]` as the program found it, which is NULL where
    /// `i` is `argc`.
    pub argument: bool,
    /// It points to bytes that the program reads or writes one by one, or
    /// to something other than `char`. A pointer to `char` without this
    /// fact only ever holds text that is passed on whole.
    pub bytes: bool,
    /// It may point into a variable of the function that holds it, which
    /// ends when the function returns.
    pub local: bool,
    /// It owns text that `getenv` read from the environment, a `String`.
    pub environment: bool,
    /// Why no rule can give it one Rust type, if so.
    pub conflict: Option<&'static str>,
}

/// How a function uses the globals that the program changes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Access {
    #[default]
    None,
    Read,
    Write,
}

/// Where a pointer value comes from.
#[derive(Clone, Debug)]
pub enum Source {
    /// The pointer that a slot holds.
    Slot(Slot),
    Null,
    /// `malloc` or `calloc`, of one element or of several.
    Fresh {
        many: bool,
    },
    /// The address of a place, or an array used as a pointer.
    Place {
        root: Root,
        array: bool,
    },
    /// A string literal.
    Literal,
    /// One of `main`'s arguments, `argv[i]`: text that the program may
    /// read and not change.
    Argument,
    /// A variable of the environment, from `getenv`: fresh text that the
    /// slot owns, or NULL.
    Environment,
    /// A function of the program or of the library, which is never NULL,
    /// or an element of an array of pointers to functions, into which no
    /// rule stores NULL.
    Function,
    /// Anything the analysis has no model for, and what it is.
    Unknown(&'static str),
}

/// What holds a place such as `s.field[2]` or `p->field`.
#[derive(Clone, Debug)]
pub enum Root {
    Var(VarId),
    /// What these pointers point to.
    Through(Vec<Source>),
    Other,
}

/// Where `Facts::in_use`, `Facts::moved`, `Facts::part_in_use` and
/// `Facts::moved_on_one_side` are.
type Found = (Option<u32>, Option<u32>, Option<u32>, Option<u32>);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Facts {
    pointers: HashMap<Slot, Pointer>,
    mutated: HashSet<VarId>,
    through: HashSet<VarId>,
    addressed: HashSet<VarId>,
    access: Vec<Access>,
    /// How the functions whose address is taken use the globals, by their
    /// C function type.
    classes: HashMap<Type, Access>,
    /// The functions that take the globals only because another function
    /// of their type needs them.
    idle: HashSet<FuncId>,
    /// Where each function first uses a local pointer to one of its
    /// variables after the variable itself was used meanwhile, and first
    /// reads a local owner after its memory went to another owner, and
    /// first uses a local pointer to a part of a record after the record
    /// was used meanwhile, and first names an owner that one side of a
    /// conditional moved.
    in_use: HashMap<FuncId, Found>,
    /// The local pointers whose use `in_use` finds.
    borrowers: HashSet<VarId>,
    /// False for the facts before the first round, which know nothing.
    settled: bool,
}

/// Library functions that store through a pointer argument, with the first
/// such argument and whether all later ones are stored through too.
const STORES: [(&str, usize, bool); 20] = [
    ("scanf", 1, true),
    ("sscanf", 2, true),
    ("fscanf", 2, true),
    ("strcpy", 0, false),
    ("strncpy", 0, false),
    ("strcat", 0, false),
    ("strncat", 0, false),
    ("memcpy", 0, false),
    ("memmove", 0, false),
    ("memset", 0, false),
    ("sprintf", 0, false),
    ("snprintf", 0, false),
    ("fgets", 0, false),
    ("fread", 0, false),
    ("strtol", 1, false),
    ("strtoll", 1, false),
    ("strtoul", 1, false),
    ("strtoull", 1, false),
    ("strtod", 1, false),
    ("strtof", 1, false),
];

/// The library functions that become a format macro, which borrows its
/// arguments for the whole call.
const FORMATS: [&str; 4] = ["printf", "fprintf", "sprintf", "snprintf"];

/// The library functions whose result is fresh memory that `free` takes.
pub const ALLOCATORS: [&str; 2] = ["malloc", "calloc"];

/// The library functions that read the text their `char` pointers point to
/// as a whole and keep none of them, so that the text can be a `&str`.
const TEXT_READERS: [&str; 22] = [
    "printf", "fprintf", "puts", "fputs", "strlen", "atoi", "atol", "atoll", "atof", "strtol",
    "strtoll", "strtoul", "strtoull", "strtod", "strtof", "strcmp", "strncmp", "getenv", "strchr",
    "strrchr", "strstr", "strpbrk",
];

impl Facts {
    /// Analyses the program. A test that the facts decide, such as `!p` for
    /// a pointer that is never NULL, makes one branch dead, and what dead
    /// code does is not a fact, so the analysis repeats until the facts
    /// stay the same.
    pub fn of(program: &Program) -> Facts {
        let mut facts = Facts::default();
        // Each round can only find more dead code, so the rounds end; the
        // limit guards against a mistake in that argument.
        for _ in 0..100 {
            let next = Walk::new(program, &facts).run();
            if next == facts {
                break;
            }
            facts = next;
        }
        facts
    }

    pub fn pointer(&self, slot: Slot) -> Pointer {
        self.pointers.get(&slot).copied().unwrap_or_default()
    }

    /// Whether the variable changes after its declaration: it is assigned,
    /// or stored to through its address.
    pub fn mutated(&self, var: VarId) -> bool {
        self.mutated.contains(&var)
    }

    /// Whether the program stores through the pointer that this variable
    /// itself holds, rather than through one it was handed on to. An owner
    /// and an `Option` then need a binding that is `mut`.
    pub fn stored_through(&self, var: VarId) -> bool {
        self.through.contains(&var)
    }

    /// Whether the program takes the address of the variable or of a part
    /// of it, so that it may change where no assignment names it.
    pub fn addressed(&self, var: VarId) -> bool {
        self.addressed.contains(&var)
    }

    /// How the function, with everything it calls, uses the globals that
    /// the program changes.
    pub fn access(&self, function: FuncId) -> Access {
        self.access.get(function).copied().unwrap_or_default()
    }

    /// How a call through a pointer to a function of this C type uses the
    /// globals: as the strongest of the functions whose address is taken
    /// and that have the type.
    pub fn class_access(&self, function: &Type) -> Access {
        self.classes.get(function).copied().unwrap_or_default()
    }

    /// Whether the function takes the globals and uses them nowhere, as
    /// it has them only for the other functions of its type.
    pub fn idle(&self, function: FuncId) -> bool {
        self.idle.contains(&function)
    }

    /// Where the function first uses a local pointer to one of its
    /// variables after that variable was stored to since the pointer got
    /// its address, or read while the pointer is stored through: Rust
    /// rejects the borrow that the pointer is.
    pub fn in_use(&self, function: FuncId) -> Option<u32> {
        self.in_use.get(&function).and_then(|found| found.0)
    }

    /// Whether `in_use` finds a use of this local pointer.
    pub fn in_use_by(&self, pointer: VarId) -> bool {
        self.borrowers.contains(&pointer)
    }

    /// Where the function first reads a local owner after it gave its
    /// memory to another owner, a field, a global or a parameter, and
    /// before anything stored to it: Rust moved the `Box` away.
    pub fn moved(&self, function: FuncId) -> Option<u32> {
        self.in_use.get(&function).and_then(|found| found.1)
    }

    /// Where the function first names an owner again after one side of a
    /// conditional gave its memory away, as `take(k ? a : b)`: Rust may
    /// have moved it, and does not know that a later test agrees.
    pub fn moved_on_one_side(&self, function: FuncId) -> Option<u32> {
        self.in_use.get(&function).and_then(|found| found.3)
    }

    /// Where the function first uses a local pointer that borrows a part
    /// of a record, `t = a->next`, after `a` was stored to or through, or
    /// read or handed on while the pointer is stored through: Rust rejects
    /// the borrow of `a` that the pointer is.
    pub fn part_in_use(&self, function: FuncId) -> Option<u32> {
        self.in_use.get(&function).and_then(|found| found.2)
    }

    /// The value of a condition that the facts decide: a constant, or a
    /// test of a pointer that is never NULL or always NULL.
    pub fn truth(&self, program: &Program, cond: &Expr) -> Option<bool> {
        let truth = |cond| self.truth(program, cond);
        match &cond.kind {
            ExprKind::Int(value) => Some(*value != 0),
            ExprKind::Unary(UnOp::Not, inner) => truth(inner).map(|value| !value),
            // The right side decides nothing alone: the left side still runs.
            ExprKind::Binary(BinOp::And, left, right) => match truth(left)? {
                true => truth(right),
                false => Some(false),
            },
            ExprKind::Binary(BinOp::Or, left, right) => match truth(left)? {
                true => Some(true),
                false => truth(right),
            },
            ExprKind::Binary(op @ (BinOp::Eq | BinOp::Ne), left, right)
                if left.ty.is_pointer() && right.ty.is_pointer() =>
            {
                let equal = match (self.null(program, left)?, self.null(program, right)?) {
                    (true, true) => true,
                    (true, false) | (false, true) => false,
                    // Two pointers that are not NULL may still be equal.
                    (false, false) => return None,
                };
                Some(equal == (*op == BinOp::Eq))
            }
            ExprKind::Cast(inner) if inner.ty.is_pointer() && !cond.ty.is_pointer() => {
                self.null(program, inner).map(|null| !null)
            }
            _ if cond.ty.is_pointer() => self.null(program, cond).map(|null| !null),
            ExprKind::Cond(test, then, other) => match truth(test)? {
                true => truth(then),
                false => truth(other),
            },
            // A comparison of two constants, which Rust's lints refuse
            // where the two are equal.
            ExprKind::Binary(op, left, right) if op.is_comparison() => {
                if let Some(value) = cond.constant(program) {
                    return Some(value != 0);
                }
                // A comparison that the type of one side decides, as
                // `u < 0` for an unsigned `u`: the lints refuse it too.
                let (value, op, constant) = match (left.constant(program), right.constant(program))
                {
                    (None, Some(constant)) => (left, *op, constant),
                    (Some(constant), None) => (right, op.mirrored(), constant),
                    _ => return None,
                };
                Expr::limited(op, value.range()?, constant)?.ok()
            }
            // Any other constant, as `if (2 - 2)`: tested against zero, the
            // lints refuse it where it is zero.
            _ if cond.ty.is_integer() => cond.constant(program).map(|value| value != 0),
            _ => None,
        }
    }

    /// Whether a pointer is certainly NULL, or certainly not.
    fn null(&self, program: &Program, pointer: &Expr) -> Option<bool> {
        if !self.settled {
            return None;
        }
        let sources = self.sources(program, pointer);
        if sources.iter().all(|source| matches!(source, Source::Null)) {
            return Some(true);
        }
        let never = sources.iter().all(|source| match source {
            Source::Slot(slot) => {
                let pointer = self.pointer(*slot);
                !pointer.nullable && !pointer.argument
            }
            Source::Fresh { .. } | Source::Place { .. } | Source::Literal | Source::Function => {
                true
            }
            // `argv[argc]` is NULL, and the analysis does not know `argc`.
            Source::Null | Source::Argument | Source::Environment | Source::Unknown(_) => false,
        });
        never.then_some(false)
    }

    /// Where the value of a pointer expression may come from.
    pub fn sources(&self, program: &Program, pointer: &Expr) -> Vec<Source> {
        let sources = |pointer| self.sources(program, pointer);
        match &pointer.kind {
            ExprKind::Null => vec![Source::Null],
            ExprKind::Var(_) | ExprKind::Member { .. } => match place_slot(pointer) {
                Some(slot) => vec![Source::Slot(slot)],
                None => vec![Source::Unknown(
                    "a pointer in a place the analysis does not follow",
                )],
            },
            ExprKind::Call(callee, args) => match callee.function() {
                Some(name) => match program.function(name) {
                    Some(function) => vec![Source::Slot(Slot::Return(function))],
                    None if ALLOCATORS.contains(&name) => vec![Source::Fresh {
                        many: !allocates_one(program, name, args, None),
                    }],
                    None if name == "realloc" => grown(program, args),
                    None if name == "getenv" => vec![Source::Environment],
                    None if crate::normalise::SEARCHES.contains(&name) => vec![Source::Unknown(
                        "a place that a search found, used as a pointer: no rule yet",
                    )],
                    None => vec![Source::Unknown("a pointer from a library function")],
                },
                None => vec![Source::Unknown("a pointer from a call through a pointer")],
            },
            ExprKind::Cast(inner) => match (&inner.kind, inner.ty.is_pointer()) {
                // The cast says what `malloc` allocates.
                (ExprKind::Call(callee, args), _)
                    if callee
                        .function()
                        .is_some_and(|name| ALLOCATORS.contains(&name)) =>
                {
                    let name = callee.function().unwrap_or_default();
                    vec![Source::Fresh {
                        many: !allocates_one(program, name, args, pointer.ty.element()),
                    }]
                }
                // R59: what `realloc` returns is the array it grew, which
                // the rule keeps in the slot that held it.
                (ExprKind::Call(callee, args), _) if callee.function() == Some("realloc") => {
                    grown(program, args)
                }
                (_, true) => sources(inner),
                (_, false) => vec![Source::Unknown("a pointer made from a number")],
            },
            // The address of the element at a cursor is a position in the
            // array, which may be its end, where there is no element.
            ExprKind::AddrOf(place) => vec![Source::Place {
                root: self.root(program, place),
                array: matches!(&place.kind, ExprKind::Index(_, index) if index.ty == Type::Index),
            }],
            ExprKind::Decay(array) => match array.kind {
                ExprKind::Str(_) => vec![Source::Literal],
                _ => vec![Source::Place {
                    root: self.root(program, array),
                    array: true,
                }],
            },
            ExprKind::Binary(BinOp::Add | BinOp::Sub, left, right) => {
                sources(if left.ty.is_pointer() { left } else { right })
            }
            ExprKind::Cond(cond, then, other) => match self.truth(program, cond) {
                Some(true) => sources(then),
                Some(false) => sources(other),
                None => [sources(then), sources(other)].concat(),
            },
            ExprKind::Comma(_, last) => sources(last),
            ExprKind::Assign { target, .. } | ExprKind::IncDec { target, .. } => {
                match place_slot(target) {
                    Some(slot) => vec![Source::Slot(slot)],
                    None => vec![Source::Unknown(
                        "a pointer in a place the analysis does not follow",
                    )],
                }
            }
            ExprKind::Index(base, _) if is_argv(program, base) => vec![Source::Argument],
            ExprKind::Index(..) if pointer.ty.function().is_some() => vec![Source::Function],
            ExprKind::Index(..) | ExprKind::Deref(_) => match place_slot(pointer) {
                Some(slot) => vec![Source::Slot(slot)],
                None => vec![Source::Unknown(
                    "a pointer stored in an array or behind a pointer",
                )],
            },
            ExprKind::Function(_) => vec![Source::Function],
            _ => vec![Source::Unknown("a pointer the analysis has no model for")],
        }
    }

    /// What holds the place.
    pub fn root(&self, program: &Program, place: &Expr) -> Root {
        match &place.kind {
            ExprKind::Var(var) => Root::Var(*var),
            ExprKind::Member {
                base, arrow: false, ..
            } => self.root(program, base),
            ExprKind::Member {
                base, arrow: true, ..
            } => Root::Through(self.sources(program, base)),
            ExprKind::Index(base, _) => match &base.kind {
                ExprKind::Decay(array) => self.root(program, array),
                _ => Root::Through(self.sources(program, base)),
            },
            ExprKind::Deref(pointer) => Root::Through(self.sources(program, pointer)),
            _ => Root::Other,
        }
    }
}

/// Whether the expression is `argv`, the second parameter of `main`.
pub fn is_argv(program: &Program, expr: &Expr) -> bool {
    let argv = program
        .function("main")
        .and_then(|main| program.functions[main].params.get(1));
    matches!(expr.kind, ExprKind::Var(var) if Some(&var) == argv)
}

/// The library functions that read a number from the text of their first
/// argument and store where it ends through their second.
pub const PARSERS: [&str; 6] = [
    "strtol", "strtoll", "strtoul", "strtoull", "strtod", "strtof",
];

/// The slot of a place that holds a pointer, if the analysis follows it.
pub fn place_slot(place: &Expr) -> Option<Slot> {
    if !place.ty.is_pointer() {
        return None;
    }
    match &place.kind {
        ExprKind::Var(var) => Some(Slot::Var(*var)),
        ExprKind::Member { record, field, .. } => Some(Slot::Field(*record, *field)),
        // `*pp` and `pp[i]`, for a variable or a field `pp`.
        ExprKind::Deref(pointer) | ExprKind::Index(pointer, _)
            if matches!(pointer.kind, ExprKind::Var(_) | ExprKind::Member { .. }) =>
        {
            place_slot(pointer)?.behind()
        }
        // R67: `a[i]` of an array of pointers, a variable or a field.
        ExprKind::Index(base, _) if place.ty.function().is_none() => match &base.kind {
            ExprKind::Decay(array) => array_slot(array)?.behind(),
            _ => None,
        },
        _ => None,
    }
}

/// The slot whose elements an array of pointers to data holds: the
/// variable or the field that the array is.
pub fn array_slot(array: &Expr) -> Option<Slot> {
    let pointers = |ty: &Type| matches!(ty, Type::Array(..)) && to_pointer(ty);
    if !pointers(&array.ty) {
        return None;
    }
    match array.kind {
        ExprKind::Var(var) => Some(Slot::Var(var)),
        ExprKind::Member { record, field, .. } => Some(Slot::Field(record, field)),
        _ => None,
    }
}

/// The slot of a place of type `ty` that holds pointers to data, itself
/// or as the elements of an array of them.
pub fn holder(ty: &Type, slot: Slot) -> Option<Slot> {
    let elements = matches!(ty, Type::Array(..)) && to_pointer(ty);
    (ty.is_pointer() || elements).then_some(slot)
}

/// The slot of the elements of an array of type `ty` whose own slot is
/// `slot`, where they are pointers to data.
pub fn elements(ty: &Type, slot: Option<Slot>) -> Option<Slot> {
    match ty {
        Type::Array(..) if to_pointer(ty) => slot?.behind(),
        _ => None,
    }
}

/// The library function that allocates the pointer's memory, if the
/// expression is a call of `malloc` or `calloc`.
fn allocator(pointer: &Expr) -> Option<&'static str> {
    match &pointer.uncast().kind {
        ExprKind::Call(callee, _) => {
            let name = callee.function()?;
            ALLOCATORS.into_iter().find(|&allocator| allocator == name)
        }
        _ => None,
    }
}

/// Whether the pointer points to a place that holds a pointer to data.
pub fn to_pointer(pointer: &Type) -> bool {
    pointer
        .element()
        .is_some_and(|element| element.is_pointer() && element.function().is_none())
}

/// The pointer through which the place is reached, if it is reached
/// through one, as `p` in `p->field[1]`.
fn dereferenced(place: &Expr) -> Option<&Expr> {
    match &place.kind {
        ExprKind::Member {
            base, arrow: true, ..
        }
        | ExprKind::Deref(base) => Some(base),
        ExprKind::Member {
            base, arrow: false, ..
        } => dereferenced(base),
        ExprKind::Index(base, _) => match &base.kind {
            ExprKind::Decay(array) => dereferenced(array),
            _ => Some(base),
        },
        _ => None,
    }
}

/// Whether the allocation holds one element of `element`, as far as the
/// arguments say.
fn allocates_one(program: &Program, name: &str, args: &[Expr], element: Option<&Type>) -> bool {
    let size = element
        .and_then(|element| element.layout(program))
        .map(|(size, _)| i128::from(size));
    let one = |bytes: &Expr| match size {
        Some(size) => bytes.constant(program) == Some(size),
        None => matches!(bytes.uncast().kind, ExprKind::SizeOf(_)),
    };
    match (name, args) {
        ("malloc", [bytes]) => one(bytes),
        ("calloc", [count, bytes]) => count.constant(program) == Some(1) && one(bytes),
        _ => false,
    }
}

struct Flow {
    from: Source,
    to: Slot,
    /// The function the flow is in, if any.
    function: Option<FuncId>,
    /// What holds the pointer, for one read from a record's field or from
    /// a global.
    holder: Option<Root>,
    /// Whether a test has found the pointer not to be NULL.
    set: bool,
}

/// One round of the analysis, which reads the conditions that the facts of
/// the round before decide.
struct Walk<'a> {
    program: &'a Program,
    known: &'a Facts,
    function: Option<FuncId>,
    flows: Vec<Flow>,
    /// The slots that are one memory: a slot behind a pointer and a place
    /// it names, or the slots behind two pointers that one value reaches.
    /// `None` for a place the analysis does not follow.
    joins: Vec<(Slot, Option<Slot>)>,
    /// The local pointers declared without a value.
    unset: Vec<(VarId, Option<FuncId>)>,
    pointers: HashMap<Slot, Pointer>,
    mutated: HashSet<VarId>,
    through: HashSet<VarId>,
    addressed: HashSet<VarId>,
    /// The functions that change a global themselves.
    writers: HashSet<FuncId>,
    /// The globals each function names.
    named: Vec<HashSet<VarId>>,
    calls: Vec<(FuncId, FuncId)>,
    /// The functions whose address is taken, with their C function type.
    taken: Vec<(FuncId, Type)>,
    /// The calls through a pointer to a function of a C type.
    indirect: Vec<(FuncId, Type)>,
}

impl<'a> Walk<'a> {
    fn new(program: &'a Program, known: &'a Facts) -> Self {
        Walk {
            program,
            known,
            function: None,
            flows: Vec::new(),
            joins: Vec::new(),
            unset: Vec::new(),
            pointers: HashMap::new(),
            mutated: HashSet::new(),
            through: HashSet::new(),
            addressed: HashSet::new(),
            writers: HashSet::new(),
            named: vec![HashSet::new(); program.functions.len()],
            calls: Vec::new(),
            taken: Vec::new(),
            indirect: Vec::new(),
        }
    }

    fn run(mut self) -> Facts {
        let program = self.program;
        // What a record or a global holds lives on after the function that
        // stored it, so a pointer there owns what it points to: Rust has no
        // borrow that a rule here could give it.
        for (id, record) in program.records.iter().enumerate() {
            for (index, field) in record.fields.iter().enumerate() {
                // A pointer to a function points to no memory.
                if field.ty.is_pointer() && field.ty.function().is_none() {
                    self.seed(Slot::Field(id, index)).owned = true;
                }
            }
        }
        for &global in &program.globals {
            let var = &program.vars[global];
            if var.ty.is_pointer() && var.ty.function().is_none() {
                self.seed(Slot::Var(global)).owned = true;
            }
            let slot = holder(&var.ty, Slot::Var(global));
            match &var.init {
                Some(init) => {
                    self.init(&var.ty, slot, init);
                    self.expr(init);
                }
                // C starts a global without an initialiser at zero.
                None => self.zero(&var.ty, slot),
            }
        }
        for (id, function) in program.functions.iter().enumerate() {
            self.function = Some(id);
            self.stmts(&function.body);
        }
        self.function = None;
        // Reading a local pointer that the program never assigns is
        // undefined, so any value is right for it, and NULL is the one
        // that Rust can write.
        for (var, function) in std::mem::take(&mut self.unset) {
            if !self.mutated.contains(&var) {
                self.flows.push(Flow {
                    from: Source::Null,
                    to: Slot::Var(var),
                    function,
                    holder: None,
                    set: false,
                });
            }
        }
        self.solve();
        let state: HashSet<VarId> = program
            .globals
            .iter()
            .copied()
            .filter(|global| self.mutated.contains(global))
            .collect();
        let mut access: Vec<Access> = (0..program.functions.len())
            .map(|function| {
                if self.writers.contains(&function) {
                    Access::Write
                } else if !self.named[function].is_disjoint(&state) {
                    Access::Read
                } else {
                    Access::None
                }
            })
            .collect();
        let own = access.clone();
        // A function uses the globals at least as its callees do. The
        // functions that a pointer of one type may reach take the same
        // `globals`, the strongest that one of them needs, and a call
        // through such a pointer passes it.
        let mut classes: HashMap<Type, Access> = HashMap::new();
        loop {
            let mut changed = false;
            let mut raise = |at: &mut Access, to: Access| {
                if *at < to {
                    *at = to;
                    changed = true;
                }
            };
            for &(caller, callee) in &self.calls {
                let to = access[callee];
                raise(&mut access[caller], to);
            }
            for (function, ty) in &self.taken {
                raise(classes.entry(ty.clone()).or_default(), access[*function]);
            }
            for (function, ty) in &self.taken {
                raise(&mut access[*function], classes[ty]);
            }
            for (caller, ty) in &self.indirect {
                let to = classes.get(ty).copied().unwrap_or_default();
                raise(&mut access[*caller], to);
            }
            if !changed {
                break;
            }
        }
        let idle = (0..program.functions.len())
            .filter(|&function| {
                access[function] != Access::None
                    && own[function] == Access::None
                    && self.calls.iter().all(|&(caller, callee)| {
                        caller != function || access[callee] == Access::None
                    })
                    && self.indirect.iter().all(|(caller, ty)| {
                        *caller != function || classes.get(ty).is_none_or(|&to| to == Access::None)
                    })
            })
            .collect();
        let mut borrowers = HashSet::new();
        let in_use = program
            .functions
            .iter()
            .enumerate()
            .map(|(id, function)| {
                let mut walk = InUse {
                    pointers: &self.pointers,
                    program,
                    found: None,
                    moved: None,
                    part: None,
                    split: None,
                    borrowers: &mut borrowers,
                };
                walk.stmts(&function.body, &mut Some(Order::default()));
                (id, (walk.found, walk.moved, walk.part, walk.split))
            })
            .collect();
        Facts {
            in_use,
            borrowers,
            pointers: self.pointers,
            mutated: self.mutated,
            through: self.through,
            addressed: self.addressed,
            access,
            classes,
            idle,
            settled: true,
        }
    }

    fn stmts(&mut self, stmts: &[Stmt]) {
        for stmt in stmts {
            self.stmt(stmt);
        }
    }

    fn stmt(&mut self, stmt: &Stmt) {
        let program = self.program;
        match stmt {
            Stmt::Expr(expr) => self.expr(expr),
            Stmt::Decl(id) => {
                let var = &program.vars[*id];
                let slot = holder(&var.ty, Slot::Var(*id));
                if let Some(len) = &var.len {
                    self.expr(len);
                }
                match &var.init {
                    Some(init) => {
                        self.init(&var.ty, slot, init);
                        self.expr(init);
                    }
                    None if var.kind.is_global() => self.zero(&var.ty, slot),
                    None => {
                        if var.ty.is_pointer() {
                            self.unset.push((*id, self.function));
                        }
                    }
                }
            }
            Stmt::Block(body) => self.stmts(body),
            Stmt::If(cond, then, other) => {
                self.expr(cond);
                let truth = self.known.truth(program, cond);
                if truth != Some(false) {
                    self.stmts(then);
                }
                if truth != Some(true) {
                    self.stmts(other);
                }
            }
            Stmt::While(cond, body) => {
                self.expr(cond);
                if self.known.truth(program, cond) != Some(false) {
                    self.stmts(body);
                }
            }
            Stmt::DoWhile(body, cond) => {
                self.stmts(body);
                self.expr(cond);
            }
            Stmt::For {
                init,
                cond,
                step,
                body,
            } => {
                self.stmts(init);
                for part in cond.iter().chain(step) {
                    self.expr(part);
                }
                self.stmts(body);
            }
            Stmt::Switch(value, body) => {
                self.expr(value);
                self.stmts(body);
            }
            Stmt::Return(Some(value)) => {
                if let (Some(function), true) = (self.function, value.ty.is_pointer()) {
                    self.flow(value, Slot::Return(function));
                }
                self.expr(value);
            }
            Stmt::IfSome {
                var,
                source,
                then,
                other,
            } => {
                // A place that a search found (R47) is an index, no pointer.
                if source.ty != Type::Found {
                    self.flow_of(source, Slot::Var(*var), true);
                }
                self.expr(source);
                self.stmts(then);
                self.stmts(other);
            }
            Stmt::LetSome {
                var,
                source,
                otherwise,
            } => {
                if source.ty != Type::Found {
                    self.flow_of(source, Slot::Var(*var), true);
                }
                self.expr(source);
                self.stmts(otherwise);
            }
            Stmt::Parse { call, start, .. } => {
                self.expr(call);
                if let Some((start, _)) = start {
                    self.expr(start);
                }
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

    /// Records what an expression does. It is called once for every
    /// expression that may run.
    fn expr(&mut self, expr: &Expr) {
        let program = self.program;
        match &expr.kind {
            ExprKind::Var(var) => {
                if let (Some(function), true) = (self.function, program.vars[*var].kind.is_global())
                {
                    self.named[function].insert(*var);
                }
            }
            ExprKind::Assign { op, target, value } => {
                self.write(target);
                if target.ty.is_pointer() {
                    match (op, place_slot(target)) {
                        (None, Some(slot)) => self.flow(value, slot),
                        // `p += n` moves within an array.
                        (Some(_), Some(slot)) => self.moves(slot),
                        (_, None) => {}
                    }
                }
            }
            ExprKind::IncDec { target, .. } => {
                self.write(target);
                if let Some(slot) = place_slot(target) {
                    self.moves(slot);
                }
            }
            // R69: the number is made from an array of bytes.
            ExprKind::Deref(pointer) if expr.number_in_bytes().is_some() => self.many(pointer),
            ExprKind::Index(base, index) => {
                if index.constant(program) != Some(0) || expr.number_in_bytes().is_some() {
                    self.many(base);
                }
                self.bytes(base);
            }
            ExprKind::AddrOf(place) => {
                if let Some(var) = place.root_var() {
                    self.addressed.insert(var);
                }
            }
            ExprKind::Binary(BinOp::Add | BinOp::Sub, left, right) => {
                // An index beside text is where a number ends in it, and
                // the text stays text.
                for (side, other) in [(left, right), (right, left)] {
                    if side.ty.is_pointer() && other.ty != Type::Index {
                        self.many(side);
                    }
                }
            }
            ExprKind::Function(name) => {
                if let (Some(function), Some(ty)) = (program.function(name), expr.ty.function()) {
                    self.taken.push((function, ty.clone()));
                }
            }
            ExprKind::Call(callee, args) => match callee.function() {
                // A function called by name has its address taken nowhere.
                Some(name) => {
                    self.call(name, args);
                    for arg in args {
                        self.expr(arg);
                    }
                    return;
                }
                None => {
                    if let (Some(caller), Some(ty)) = (self.function, callee.ty.function()) {
                        self.indirect.push((caller, ty.clone()));
                    }
                }
            },
            // `calloc` starts its memory at zero, so the pointers in it
            // are NULL.
            ExprKind::Cast(inner) => {
                if let ExprKind::Call(callee, _) = &inner.kind
                    && callee.function() == Some("calloc")
                    && let Some(element) = expr.ty.element()
                {
                    self.zero(element, None);
                }
            }
            // R35: the statements of a statement expression run as those
            // of the function do.
            ExprKind::Block { stmts, value } => {
                self.stmts(stmts);
                if let Some(value) = value {
                    self.expr(value);
                }
                return;
            }
            ExprKind::Init { var, value } => {
                let ty = &program.vars[*var].ty;
                let slot = holder(ty, Slot::Var(*var));
                // The declaration before it has no first value.
                self.unset.retain(|(unset, _)| unset != var);
                self.init(ty, slot, value);
            }
            ExprKind::Cond(cond, then, other) => {
                self.expr(cond);
                let truth = self.known.truth(program, cond);
                if truth != Some(false) {
                    self.expr(then);
                }
                if truth != Some(true) {
                    self.expr(other);
                }
                return;
            }
            _ => {}
        }
        for child in expr.children() {
            self.expr(child);
        }
    }

    fn call(&mut self, name: &str, args: &[Expr]) {
        let program = self.program;
        if let Some(callee) = program.function(name) {
            if let Some(caller) = self.function {
                self.calls.push((caller, callee));
            }
            for (&param, arg) in program.functions[callee].params.iter().zip(args) {
                if program.vars[param].ty.is_pointer() {
                    self.flow(arg, Slot::Var(param));
                }
            }
            return;
        }
        // `realloc` frees the array it grows, as `free` does.
        if let ("free" | "realloc", Some(freed)) = (name, args.first()) {
            for source in self.sources(freed) {
                if let Source::Slot(slot) = source {
                    self.seed(slot).owned = true;
                }
            }
            // What a field owned is taken out of its record, and what a
            // place behind a pointer owned out of that place.
            if matches!(
                freed.uncast().kind,
                ExprKind::Member { .. } | ExprKind::Deref(_) | ExprKind::Index(..)
            ) && place_slot(freed.uncast()).is_some()
            {
                self.write(freed.uncast());
            }
        }
        // The end pointer points into the text that was read, which was
        // therefore not NULL.
        if let (true, [text, end, ..]) = (PARSERS.contains(&name), args)
            && let ExprKind::AddrOf(place) = &end.uncast().kind
            && let Some(slot) = place_slot(place)
        {
            self.flow_of(text, slot, true);
        }
        // `sprintf` and `snprintf` store bytes into their first argument
        // and read the others as text.
        let text_from = match name {
            "sprintf" | "snprintf" => 1,
            _ if TEXT_READERS.contains(&name) => 0,
            _ => args.len(),
        };
        for (index, arg) in args.iter().enumerate() {
            if !arg.ty.is_pointer() {
                continue;
            }
            // Text is an array of `char` up to its NUL, and so are the
            // bytes that the library reads or stores.
            if arg.ty.element() == Some(&Type::Char) {
                for source in self.sources(arg) {
                    if let Source::Slot(slot) = source {
                        self.seed(slot).many = true;
                    }
                }
            }
            if index < text_from {
                self.bytes(arg);
            }
        }
        let stores = STORES.iter().find(|(function, ..)| *function == name);
        if let Some(&(_, first, rest)) = stores {
            for arg in args
                .iter()
                .skip(first)
                .take(if rest { usize::MAX } else { 1 })
            {
                if arg.ty.is_pointer() {
                    let sources = self.sources(arg);
                    self.write_through(&sources, self.function);
                }
            }
        }
    }

    /// Records what an initialiser stores in a place of type `ty`, whose
    /// slot is `slot` if it holds a pointer.
    fn init(&mut self, ty: &Type, slot: Option<Slot>, init: &Expr) {
        let program = self.program;
        match (ty, &init.kind) {
            (_, ExprKind::Zero) => self.zero(ty, slot),
            (Type::Pointer(_), _) => {
                if let Some(slot) = slot {
                    self.flow(init, slot);
                }
            }
            (Type::Record(record), ExprKind::InitList(items)) => {
                for (index, field) in program.records[*record].fields.iter().enumerate() {
                    let slot = holder(&field.ty, Slot::Field(*record, index));
                    match items.get(index) {
                        Some(item) => self.init(&field.ty, slot, item),
                        None => self.zero(&field.ty, slot),
                    }
                }
            }
            (Type::Array(element, length), ExprKind::InitList(items)) => {
                let slot = elements(ty, slot);
                for item in items {
                    self.init(element, slot, item);
                }
                if length.is_none_or(|length| (items.len() as u64) < length) {
                    self.zero(element, slot);
                }
            }
            _ => {}
        }
    }

    /// Records that a place of type `ty` starts at zero: its pointers are
    /// NULL.
    fn zero(&mut self, ty: &Type, slot: Option<Slot>) {
        let program = self.program;
        match ty {
            Type::Pointer(_) => {
                if let Some(slot) = slot {
                    self.flows.push(Flow {
                        from: Source::Null,
                        to: slot,
                        function: self.function,
                        holder: None,
                        set: false,
                    });
                }
            }
            Type::Record(record) => {
                for (index, field) in program.records[*record].fields.iter().enumerate() {
                    let slot = holder(&field.ty, Slot::Field(*record, index));
                    self.zero(&field.ty, slot);
                }
            }
            Type::Array(element, _) => self.zero(element, elements(ty, slot)),
            _ => {}
        }
    }

    fn sources(&self, pointer: &Expr) -> Vec<Source> {
        self.known.sources(self.program, pointer)
    }

    fn seed(&mut self, slot: Slot) -> &mut Pointer {
        self.pointers.entry(slot).or_default()
    }

    /// Records that the slot's pointer moves within an array.
    fn moves(&mut self, slot: Slot) {
        let moved = self.seed(slot);
        (moved.many, moved.bytes) = (true, true);
        if matches!(slot, Slot::Behind(_)) {
            moved.conflict = Some("a cursor behind a pointer: no rule yet");
        }
    }

    /// The slot that a pointer to a pointer names: the place whose address
    /// it is, or the slot behind the pointer it copies. `None` for any
    /// other, NULL included, which names nothing and is not followed.
    fn named(&self, pointer: &Expr) -> Option<Slot> {
        let pointer = pointer.uncast();
        match &pointer.kind {
            // `argv` is the program's arguments, which no slot holds.
            _ if is_argv(self.program, pointer) => None,
            ExprKind::AddrOf(place) => place_slot(place),
            ExprKind::Decay(array) => array_slot(array)?.behind(),
            ExprKind::Var(_) | ExprKind::Member { .. } | ExprKind::Deref(_) => {
                place_slot(pointer)?.behind()
            }
            _ => None,
        }
    }

    /// Records that the pointer's value goes into the slot.
    fn flow(&mut self, pointer: &Expr, to: Slot) {
        self.flow_of(pointer, to, false);
    }

    /// Records that the pointer's value goes into the slot; `set` says that
    /// a test has found it not to be NULL.
    fn flow_of(&mut self, pointer: &Expr, to: Slot, set: bool) {
        let held = pointer.uncast();
        // Each value of a conditional is held by its own place.
        match &held.kind {
            ExprKind::Cond(cond, then, other) if self.known.truth(self.program, cond).is_none() => {
                self.flow_of(then, to, set);
                self.flow_of(other, to, set);
                return;
            }
            ExprKind::Comma(_, last) => return self.flow_of(last, to, set),
            _ => {}
        }
        // A pointer to a pointer makes the places behind both ends one.
        if to_pointer(slot_type(self.program, to))
            && let Some(behind) = to.behind()
            && !matches!(held.kind, ExprKind::Null)
        {
            match allocator(held) {
                // R67: the memory of the heap is a place of its own; the
                // zero that `calloc` leaves in an element is NULL.
                Some(name) => {
                    if name == "calloc" {
                        self.flows.push(Flow {
                            from: Source::Null,
                            to: behind,
                            function: self.function,
                            holder: None,
                            set: false,
                        });
                    }
                }
                None => self.joins.push((behind, self.named(held))),
            }
        }
        let holder = match held.kind {
            ExprKind::Member { .. } | ExprKind::Deref(_) => {
                Some(self.known.root(self.program, held))
            }
            ExprKind::Var(var) if self.program.vars[var].kind.is_global() => Some(Root::Var(var)),
            // `&s->items[i]`: the element is reached through the record
            // that holds the pointer to it.
            ExprKind::AddrOf(ref place) => dereferenced(place)
                .map(Expr::uncast)
                .filter(|pointer| matches!(pointer.kind, ExprKind::Member { .. }))
                .map(|pointer| self.known.root(self.program, pointer)),
            _ => None,
        };
        for from in self.sources(pointer) {
            self.flows.push(Flow {
                from,
                to,
                function: self.function,
                holder: holder.clone(),
                set,
            });
        }
    }

    /// Records that the pointer points into an array.
    fn many(&mut self, pointer: &Expr) {
        for source in self.sources(pointer) {
            if let Source::Slot(slot) = source {
                self.seed(slot).many = true;
            }
        }
        self.bytes(pointer);
    }

    /// Records that the program reads or writes single elements through
    /// the pointer, or hands it to a function that may.
    fn bytes(&mut self, pointer: &Expr) {
        for source in self.sources(pointer) {
            if let Source::Slot(slot) = source {
                self.seed(slot).bytes = true;
            }
        }
    }

    /// Whether the slot's pointers point to `char`.
    fn holds_chars(&self, slot: Slot) -> bool {
        slot_type(self.program, slot).element() == Some(&Type::Char)
    }

    /// Records a store to the place.
    fn write(&mut self, place: &Expr) {
        let root = self.known.root(self.program, place);
        self.write_root(&root, self.function);
        // Rust reaches what a field points to through the record, so a
        // store through the field needs a writable record, and one through
        // `*pp` or `pp[i]` a writable `pp`.
        if let Some(pointer) = dereferenced(place)
            && (matches!(
                pointer.uncast().kind,
                ExprKind::Member { .. } | ExprKind::Deref(_)
            ) || matches!(pointer.uncast().kind, ExprKind::Index(..))
                && place_slot(pointer.uncast()).is_some())
        {
            self.write(pointer.uncast());
        }
    }

    fn write_root(&mut self, root: &Root, function: Option<FuncId>) {
        match root {
            Root::Var(var) => {
                self.mutated.insert(*var);
                if let (Some(function), true) = (function, self.program.vars[*var].kind.is_global())
                {
                    self.writers.insert(function);
                }
            }
            Root::Through(sources) => self.write_through(sources, function),
            Root::Other => {}
        }
    }

    /// Records a store through pointers from these sources.
    fn write_through(&mut self, sources: &[Source], function: Option<FuncId>) {
        for source in sources {
            match source {
                Source::Slot(slot) => {
                    self.seed(*slot).written = true;
                    if let Slot::Var(var) = slot {
                        self.through.insert(*var);
                    }
                }
                Source::Place { root, .. } => self.write_root(root, function),
                _ => {}
            }
        }
    }

    /// Spreads the facts along the flows until nothing changes.
    fn solve(&mut self) {
        let flows = std::mem::take(&mut self.flows);
        let joins = std::mem::take(&mut self.joins);
        loop {
            let before = (
                self.pointers.clone(),
                self.mutated.len(),
                self.through.len(),
                self.writers.len(),
            );
            for flow in &flows {
                let mut to = *self.seed(flow.to);
                // Text is never stored through or freed, and a `&str`
                // converts to no other pointer.
                to.bytes |= to.written || to.owned || !self.holds_chars(flow.to);
                match &flow.from {
                    Source::Slot(slot) => {
                        let mut from = *self.seed(*slot);
                        from.bytes |= from.written || from.owned || !self.holds_chars(*slot);
                        // Rust reaches what a field or a global points to
                        // through its holder, so the holder must be
                        // writable too. Moving what it owns out of it
                        // stores to the holder as well.
                        if let (true, Some(holder)) = (to.written || to.owned, &flow.holder) {
                            self.write_root(holder, flow.function);
                        }
                        // A borrow that is stored through is taken from
                        // this variable's own binding.
                        if let (Slot::Var(var), true) = (slot, to.written && !to.owned) {
                            self.through.insert(*var);
                        }
                        from.written |= to.written;
                        from.many |= to.many;
                        to.many |= from.many;
                        // One type holds either text or bytes, so both
                        // ends of a flow hold the same.
                        from.bytes |= to.bytes;
                        to.bytes |= from.bytes;
                        from.owned |= to.owned;
                        // A result has no other owner, and what a record,
                        // a result or a global holds outlives a local.
                        let keeps =
                            matches!(slot, Slot::Return(_)) || self.outlives_locals(flow.to);
                        to.owned |= from.owned && keeps;
                        to.environment |= from.environment && keeps;
                        // A borrow of the `String` that a local owns ends
                        // with the function, as a borrow of the local does.
                        if from.environment && !keeps {
                            self.reaches_local(flow.to, &mut to);
                        }
                        to.nullable |= from.nullable && !flow.set;
                        to.argument |= from.argument && !flow.set;
                        if from.local {
                            self.reaches_local(flow.to, &mut to);
                        }
                        *self.seed(*slot) = from;
                    }
                    Source::Null => to.nullable |= !flow.set,
                    Source::Fresh { many } => {
                        to.owned = true;
                        to.bytes = true;
                        to.many |= many;
                    }
                    Source::Place { root, array } => {
                        to.many |= array;
                        to.bytes = true;
                        if matches!(root, Root::Var(var) if !self.program.vars[*var].kind.is_global())
                        {
                            self.reaches_local(flow.to, &mut to);
                        }
                        if to.written {
                            self.write_root(root, flow.function);
                            if let Some(holder) = &flow.holder {
                                self.write_root(holder, flow.function);
                            }
                        }
                        if to.owned {
                            to.conflict = Some("it owns memory and also points into a variable");
                        }
                    }
                    Source::Literal => {
                        to.many = true;
                        if to.written || to.owned {
                            to.conflict =
                                Some("it points to a string literal that is written or freed");
                        }
                    }
                    Source::Argument => {
                        to.argument |= !flow.set;
                        if to.written || to.owned {
                            to.conflict =
                                Some("it points to a program argument that is written or freed");
                        }
                    }
                    Source::Environment => {
                        // A parameter borrows the text, which lives to the
                        // end of the call's statement.
                        to.environment |= !matches!(
                            flow.to,
                            Slot::Var(var) if self.program.vars[var].kind == VarKind::Param
                        );
                        to.nullable |= !flow.set;
                    }
                    Source::Function => {}
                    Source::Unknown(what) => to.conflict = Some(what),
                }
                *self.seed(flow.to) = to;
            }
            // A slot behind a pointer and the place it names are one
            // memory, so each has every fact of the other.
            for &(behind, named) in &joins {
                let one = *self.seed(behind);
                let Some(named) = named else {
                    self.seed(behind).conflict =
                        Some("it points to a place that the analysis does not follow");
                    continue;
                };
                let other = *self.seed(named);
                let both = Pointer {
                    written: one.written || other.written,
                    many: one.many || other.many,
                    owned: one.owned || other.owned,
                    nullable: one.nullable || other.nullable,
                    argument: one.argument || other.argument,
                    bytes: one.bytes || other.bytes,
                    local: one.local || other.local,
                    environment: one.environment || other.environment,
                    conflict: one.conflict.or(other.conflict),
                };
                *self.seed(behind) = both;
                *self.seed(named) = both;
            }
            if before
                == (
                    self.pointers.clone(),
                    self.mutated.len(),
                    self.through.len(),
                    self.writers.len(),
                )
            {
                break;
            }
        }
    }

    /// Records that a pointer into a variable of the function reaches the
    /// slot: another local may hold it, and the result may not, since the
    /// variable ends where the function returns.
    fn reaches_local(&self, slot: Slot, pointer: &mut Pointer) {
        match slot {
            Slot::Var(var) => pointer.local |= self.program.vars[var].kind == VarKind::Local,
            Slot::Return(_) => {
                pointer.conflict = Some("it points to a variable that ends with the function")
            }
            Slot::Field(..) | Slot::Behind(_) => {}
        }
    }

    /// Whether what the slot holds outlives the function: also a place
    /// behind a pointer, which may be the caller's.
    fn outlives_locals(&self, slot: Slot) -> bool {
        match slot {
            Slot::Var(var) => self.program.vars[var].kind.is_global(),
            Slot::Field(..) | Slot::Return(_) | Slot::Behind(_) => true,
        }
    }
}

/// What the statements before have done, or `None` where no path reaches
/// the statement.
type Borrows = Option<Order>;

#[derive(Clone, Default)]
struct Order {
    /// The local pointers that hold the address of a variable of the
    /// function, with those variables, and whether the variable was stored
    /// to, or read while the pointer is stored through, since the pointer
    /// got its address.
    borrows: HashMap<VarId, (HashSet<VarId>, bool)>,
    /// The local owners whose memory went to another owner, and that
    /// nothing has stored to since.
    moved: HashSet<VarId>,
    /// The local borrows that got a pointer out of a record, `t = a->f`,
    /// with the parts they borrow, and whether one of them was used since
    /// in a way the borrow forbids.
    parts: HashMap<VarId, (HashSet<Part>, bool)>,
    /// The owners that one side of a conditional gave to another owner,
    /// and that nothing has stored to since.
    split: HashSet<VarId>,
}

/// A variable and the field of it that a path names first, `a` and `f` in
/// `a->f->g`; no field where the path indexes or dereferences `a` first,
/// or `a` is a global, which Rust reaches as one field of the globals.
/// Rust tells the fields of one variable apart, so a use of `a->g` leaves
/// a borrow of `a->f` alone.
type Part = (VarId, Option<usize>);

/// Whether a use of one part touches the other.
fn overlap(one: Part, other: Part) -> bool {
    one.0 == other.0 && (one.1.is_none() || other.1.is_none() || one.1 == other.1)
}

/// A local pointer to a variable of the function is a borrow of it in
/// Rust, which the variable itself cannot be stored to while the borrow is
/// used later, nor read while it is `&mut`. This finds the first use of
/// such a pointer after such a store or read since the pointer got its
/// address. "After" is the order of the statements, the branches of an
/// `if` each from the state before it, and a loop's condition, body and
/// step twice, so that everything in a loop also follows itself.
///
/// In the same order it finds the first read of a local owner after its
/// memory went to another owner: Rust moves the `Box` there; and the first
/// use of a local borrow of a part of a record after the record was stored
/// to or through, or read or handed on while the borrow is stored through.
struct InUse<'a> {
    pointers: &'a HashMap<Slot, Pointer>,
    program: &'a Program,
    found: Option<u32>,
    moved: Option<u32>,
    part: Option<u32>,
    split: Option<u32>,
    borrowers: &'a mut HashSet<VarId>,
}

impl InUse<'_> {
    fn stmts(&mut self, stmts: &[Stmt], state: &mut Borrows) {
        for stmt in stmts {
            self.stmt(stmt, state);
        }
    }

    fn stmt(&mut self, stmt: &Stmt, state: &mut Borrows) {
        match stmt {
            Stmt::Expr(expr) => self.expr(expr, state),
            Stmt::Decl(var) => {
                if let Some(init) = &self.program.vars[*var].init {
                    self.expr(init, state);
                    self.move_into(init, Slot::Var(*var), state);
                    self.assigned(*var, init, state);
                }
            }
            Stmt::Block(body) => self.stmts(body, state),
            Stmt::If(cond, then, other) => {
                self.expr(cond, state);
                self.branches(then, other, state);
            }
            Stmt::IfSome {
                source,
                then,
                other,
                ..
            } => {
                self.expr(source, state);
                self.branches(then, other, state);
            }
            Stmt::LetSome {
                source, otherwise, ..
            } => {
                self.expr(source, state);
                self.branches(otherwise, &[], state);
            }
            Stmt::While(cond, body) => {
                for _ in 0..2 {
                    self.expr(cond, state);
                    self.stmts(body, state);
                }
            }
            Stmt::DoWhile(body, cond) => {
                for _ in 0..2 {
                    self.stmts(body, state);
                    self.expr(cond, state);
                }
            }
            Stmt::For {
                init,
                cond,
                step,
                body,
            } => {
                self.stmts(init, state);
                for _ in 0..2 {
                    cond.iter().for_each(|cond| self.expr(cond, state));
                    self.stmts(body, state);
                    step.iter().for_each(|step| self.expr(step, state));
                }
            }
            Stmt::Switch(cond, body) => {
                self.expr(cond, state);
                self.stmts(body, state);
            }
            Stmt::Parse { call, start, .. } => {
                self.expr(call, state);
                start.iter().for_each(|(start, _)| self.expr(start, state));
            }
            Stmt::Return(value) => {
                value.iter().for_each(|value| self.expr(value, state));
                *state = None;
            }
            Stmt::Goto(_) => *state = None,
            // A jump may reach a label from anywhere: what it brings is
            // not followed.
            Stmt::Label(_) => {
                state.get_or_insert_with(Order::default);
            }
            Stmt::Case(_) | Stmt::Default | Stmt::Break | Stmt::Continue => {}
        }
    }

    /// Each branch from the state before it, and after them what either
    /// leaves.
    fn branches(&mut self, then: &[Stmt], other: &[Stmt], state: &mut Borrows) {
        let mut second = state.clone();
        self.stmts(then, state);
        self.stmts(other, &mut second);
        match (state.as_mut(), second) {
            (Some(first), Some(second)) => {
                for (pointer, (targets, touched)) in second.borrows {
                    let entry = first.borrows.entry(pointer).or_default();
                    entry.0.extend(targets);
                    entry.1 |= touched;
                }
                first.moved.extend(second.moved);
                for (pointer, (records, touched)) in second.parts {
                    let entry = first.parts.entry(pointer).or_default();
                    entry.0.extend(records);
                    entry.1 |= touched;
                }
                first.split.extend(second.split);
            }
            (None, second) => *state = second,
            (Some(_), None) => {}
        }
    }

    fn expr(&mut self, expr: &Expr, state: &mut Borrows) {
        match &expr.kind {
            ExprKind::Assign { op, target, value } => {
                self.expr(value, state);
                if op.is_none()
                    && let Some(slot) = place_slot(target)
                {
                    self.move_into(value, slot, state);
                }
                match &target.kind {
                    ExprKind::Var(var) if op.is_none() => self.assigned(*var, value, state),
                    _ => match target.root_var() {
                        Some(var) => {
                            self.parts(target, state);
                            self.stored_in(var, target, state);
                        }
                        None => {
                            self.expr(target, state);
                            if let Some(pointer) = through(target) {
                                self.stored_in(pointer, target, state);
                            }
                        }
                    },
                }
            }
            ExprKind::IncDec { target, .. } => match target.root_var() {
                Some(var)
                    if !matches!(target.kind, ExprKind::Var(_)) || !target.ty.is_pointer() =>
                {
                    self.parts(target, state);
                    self.stored_in(var, target, state);
                }
                _ => {
                    self.expr(target, state);
                    if let Some(pointer) = through(target) {
                        self.stored_in(pointer, target, state);
                    }
                }
            },
            ExprKind::Var(var) => self.read(*var, expr.at, state),
            // A field read leaves a borrow of another field of the
            // variable alone.
            ExprKind::Member { base, field, .. } if matches!(base.kind, ExprKind::Var(var) if !self.program.vars[var].kind.is_global()) =>
            {
                let ExprKind::Var(var) = base.kind else {
                    unreachable!()
                };
                self.read_part((var, Some(*field)), base.at, state);
            }
            _ => {
                for child in expr.children() {
                    self.expr(child, state);
                }
                // A format macro borrows all its arguments at once, so a
                // borrow read before a use of its record in the same
                // call is still in use there.
                if let ExprKind::Call(callee, args) = &expr.kind
                    && callee
                        .function()
                        .is_some_and(|name| FORMATS.contains(&name))
                    && let Some(order) = state
                {
                    for arg in args {
                        let mut late = |inner: &Expr| {
                            matches!(inner.kind, ExprKind::Var(var)
                                if order.parts.get(&var).is_some_and(|part| part.1))
                        };
                        if arg.any(&mut late) {
                            self.part.get_or_insert(expr.at);
                        }
                    }
                }
                if let ExprKind::Call(callee, args) = &expr.kind
                    && let Some(function) = callee
                        .function()
                        .and_then(|name| self.program.function(name))
                {
                    for (&param, arg) in self.program.functions[function].params.iter().zip(args) {
                        self.move_into(arg, Slot::Var(param), state);
                        if self
                            .pointers
                            .get(&Slot::Var(param))
                            .is_some_and(|pointer| pointer.written || pointer.owned)
                            && let Some(part) = lent(self.program, arg)
                        {
                            self.handed(part, state);
                            // The address of a variable handed on to store
                            // through is a store to it for a pointer that
                            // borrows it.
                            if let ExprKind::AddrOf(place) = &arg.uncast().kind
                                && let Some(var) = place.root_var()
                            {
                                self.stored_only(var, state);
                            }
                        }
                        // R65: `&p` lent to a `T **` that is written is a
                        // store to `p`.
                        if to_pointer(&self.program.vars[param].ty)
                            && self
                                .pointers
                                .get(&Slot::Var(param))
                                .is_some_and(|pointer| pointer.written)
                            && let ExprKind::AddrOf(place) = &arg.uncast().kind
                            && let Some(var) = place.root_var()
                        {
                            self.stored(var, state);
                        }
                    }
                }
            }
        }
    }

    /// An owner, a local or a global, that is the whole value of a slot
    /// that owns gives its memory away.
    fn move_into(&mut self, value: &Expr, slot: Slot, state: &mut Borrows) {
        let owns = |slot| {
            self.pointers
                .get(&slot)
                .is_some_and(|pointer| pointer.owned)
        };
        if let ExprKind::Cond(_, then, other) = &value.uncast().kind {
            self.one_side(then, slot, state);
            self.one_side(other, slot, state);
            return;
        }
        if let (ExprKind::Var(var), Some(order)) = (&value.uncast().kind, state.as_mut()) {
            // A global gives its memory away too: it is `None` afterwards,
            // where C still reads the pointer.
            if Slot::Var(*var) != slot && owns(Slot::Var(*var)) && owns(slot) {
                order.moved.insert(*var);
            }
        }
    }

    /// An owner that a side of a conditional gives to an owning slot.
    fn one_side(&mut self, side: &Expr, slot: Slot, state: &mut Borrows) {
        let owns = |slot| {
            self.pointers
                .get(&slot)
                .is_some_and(|pointer| pointer.owned)
        };
        match (&side.uncast().kind, state.as_mut()) {
            (ExprKind::Cond(_, then, other), _) => {
                self.one_side(then, slot, state);
                self.one_side(other, slot, state);
            }
            (ExprKind::Var(var), Some(order))
                if Slot::Var(*var) != slot && owns(Slot::Var(*var)) && owns(slot) =>
            {
                order.split.insert(*var);
            }
            _ => {}
        }
    }

    /// The indices inside a place that is a variable's own part.
    fn parts(&mut self, place: &Expr, state: &mut Borrows) {
        match &place.kind {
            ExprKind::Member { base, .. } => self.parts(base, state),
            ExprKind::Index(base, index) => {
                if let ExprKind::Decay(array) = &base.kind {
                    self.parts(array, state);
                }
                self.expr(index, state);
            }
            _ => {}
        }
    }

    fn read(&mut self, var: VarId, at: u32, state: &mut Borrows) {
        self.read_part((var, None), at, state);
    }

    /// A read of the variable, or of one field of it.
    fn read_part(&mut self, part: Part, at: u32, state: &mut Borrows) {
        let var = part.0;
        let Some(order) = state else { return };
        if order.moved.contains(&var) {
            self.moved.get_or_insert(at);
        }
        if let Some((_, true)) = order.parts.get(&var) {
            self.part.get_or_insert(at);
        }
        let written = |pointer| {
            self.pointers
                .get(&Slot::Var(pointer))
                .is_some_and(|p| p.written)
        };
        for (pointer, (parts, touched)) in order.parts.iter_mut() {
            if parts.iter().any(|&other| overlap(part, other)) && written(*pointer) {
                *touched = true;
            }
        }
        if order.split.contains(&var) {
            self.split.get_or_insert(at);
        }
        let borrows = &mut order.borrows;
        if let Some((_, true)) = borrows.get(&var) {
            self.found.get_or_insert(at);
            self.borrowers.insert(var);
        }
        for (pointer, (targets, touched)) in borrows.iter_mut() {
            if targets.contains(&var)
                && self
                    .pointers
                    .get(&Slot::Var(*pointer))
                    .is_some_and(|p| p.written)
            {
                *touched = true;
            }
        }
    }

    /// A store to the place, which is the variable or reached through it.
    fn stored_in(&mut self, var: VarId, place: &Expr, state: &mut Borrows) {
        self.stored_only(var, state);
        let part = part(self.program, place).filter(|part| part.0 == var);
        self.handed(part.unwrap_or((var, None)), state);
    }

    fn stored(&mut self, var: VarId, state: &mut Borrows) {
        self.stored_only(var, state);
        self.handed((var, None), state);
    }

    fn stored_only(&mut self, var: VarId, state: &mut Borrows) {
        let Some(order) = state else { return };
        for (targets, touched) in order.borrows.values_mut() {
            if targets.contains(&var) {
                *touched = true;
            }
        }
    }

    /// A store to or through a record, or the record handed to a function
    /// that stores through it or keeps it, which a borrow of a part of it
    /// forbids.
    fn handed(&mut self, part: Part, state: &mut Borrows) {
        let Some(order) = state else { return };
        for (parts, touched) in order.parts.values_mut() {
            if parts.iter().any(|&other| overlap(part, other)) {
                *touched = true;
            }
        }
    }

    /// `var = value`: a pointer gets the variables whose address the
    /// value is, and a store to any other variable is a store.
    fn assigned(&mut self, var: VarId, value: &Expr, state: &mut Borrows) {
        let program = self.program;
        let pointer = &program.vars[var];
        if let Some(order) = state {
            order.moved.remove(&var);
            order.split.remove(&var);
        }
        // A store to a pointer is a store to what a copy of it borrows.
        self.stored(var, state);
        if !pointer.ty.is_pointer() || pointer.kind.is_global() {
            return;
        }
        // A pointer that is stored through and gets the address of a
        // variable borrows it alone: an earlier pointer to the variable
        // sees a store.
        if self
            .pointers
            .get(&Slot::Var(var))
            .is_some_and(|pointer| pointer.written)
            && let ExprKind::AddrOf(place) | ExprKind::Decay(place) = &value.uncast().kind
            && let Some(named) = place.root_var()
            && named != var
        {
            self.stored_only(named, state);
        }
        let Some(order) = state else { return };
        // A borrow that gets a pointer out of a record borrows the record,
        // and what the record borrows; a copy borrows what its source does.
        let mut parts = HashSet::new();
        let owned = self
            .pointers
            .get(&Slot::Var(var))
            .is_some_and(|pointer| pointer.owned);
        let root = match value.uncast().kind {
            ExprKind::Var(from) => Some(from),
            _ => path_root(program, value).map(|part| {
                if part.0 != var {
                    parts.insert(part);
                }
                part.0
            }),
        };
        if let Some((more, _)) = root.and_then(|root| order.parts.get(&root)) {
            parts.extend(more.iter().copied());
        }
        if parts.is_empty() || owned {
            order.parts.remove(&var);
        } else {
            order.parts.insert(var, (parts, false));
        }
        let borrows = &mut order.borrows;
        let mut targets = HashSet::new();
        addresses(program, value, &mut targets);
        // A copy of a local pointer that owns or is stored through
        // borrows that pointer, as a pointer to a variable borrows it.
        let fact = |var| self.pointers.get(&Slot::Var(var));
        if let ExprKind::Var(from) = value.uncast().kind
            && from != var
            && matches!(program.vars[from].kind, VarKind::Local | VarKind::Param)
            && fact(from).is_some_and(|from| from.written || from.owned)
            && !fact(var).is_some_and(|copy| copy.owned)
        {
            targets.insert(from);
        }
        // A copy of a pointer borrows what that pointer borrows.
        if let ExprKind::Var(from) = value.uncast().kind
            && let Some((borrowed, _)) = borrows.get(&from)
        {
            targets.extend(borrowed.iter().copied());
        }
        if targets.is_empty() {
            borrows.remove(&var);
        } else {
            borrows.insert(var, (targets, false));
        }
    }
}

/// The pointer variable that a place is reached through: `p` in `p->f`,
/// `*p`, `p[i]` and `p->in.f`.
fn through(place: &Expr) -> Option<VarId> {
    match &place.kind {
        ExprKind::Member { base, arrow, .. } => match (&base.uncast().kind, arrow) {
            (ExprKind::Var(var), true) => Some(*var),
            _ => through(base),
        },
        ExprKind::Deref(pointer) | ExprKind::Index(pointer, _) => match pointer.uncast().kind {
            ExprKind::Var(var) => Some(var),
            _ => None,
        },
        _ => None,
    }
}

/// The part that a value read out of a record is reached through: `a`
/// and `f` in `a->f`, `a->f->g` and `a.f`, `a` in `a[i].f` and
/// `(*pp)->f`.
fn path_root(program: &Program, value: &Expr) -> Option<Part> {
    let mut place = value.uncast();
    let mut field = false;
    loop {
        place = match &place.kind {
            ExprKind::Member { base, .. } => {
                field = true;
                base
            }
            ExprKind::Deref(base)
            | ExprKind::Index(base, _)
            | ExprKind::Decay(base)
            | ExprKind::Cast(base) => base,
            ExprKind::Var(_) => return field.then(|| part(program, value)).flatten(),
            _ => return None,
        }
    }
}

/// The variable at the root of a place and the field of it that the
/// place names first.
fn part(program: &Program, place: &Expr) -> Option<Part> {
    let mut place = place.uncast();
    let mut first = None;
    loop {
        place = match &place.kind {
            ExprKind::Member { base, field, .. } => {
                first = Some(*field);
                base
            }
            ExprKind::Deref(base)
            | ExprKind::Index(base, _)
            | ExprKind::Decay(base)
            | ExprKind::Cast(base) => {
                first = None;
                base
            }
            ExprKind::Var(var) => {
                let global = program.vars[*var].kind.is_global();
                return Some((*var, first.filter(|_| !global)));
            }
            _ => return None,
        }
    }
}

/// The part whose record a pointer argument lends: `a` itself, or a part
/// of it, `&a->f`.
fn lent(program: &Program, arg: &Expr) -> Option<Part> {
    match &arg.uncast().kind {
        ExprKind::AddrOf(place) => part(program, place),
        _ => part(program, arg),
    }
}

/// The variables of the function whose address the pointer value is, as
/// `&v.field`, `&a[1][3]`, or an array `a` used as a pointer.
fn addresses(program: &Program, value: &Expr, targets: &mut HashSet<VarId>) {
    match &value.kind {
        ExprKind::Cast(inner) => addresses(program, inner, targets),
        ExprKind::AddrOf(place) | ExprKind::Decay(place) => {
            if let Some(var) = place.root_var() {
                let var = &program.vars[var];
                if matches!(var.kind, VarKind::Local | VarKind::Param) && !var.ty.is_pointer() {
                    targets.extend(place.root_var());
                }
            }
        }
        ExprKind::Binary(BinOp::Add | BinOp::Sub, left, right) => addresses(
            program,
            if left.ty.is_pointer() { left } else { right },
            targets,
        ),
        _ => {}
    }
}

/// What `realloc` returns: the array it grew, and NULL where its size may
/// be 0 when the program runs, for which the reference's library frees
/// the array and returns NULL.
fn grown(program: &Program, args: &[Expr]) -> Vec<Source> {
    match args.get(1).and_then(|bytes| fixed_size(program, bytes)) {
        Some(_) => vec![Source::Fresh { many: true }],
        None => vec![Source::Fresh { many: true }, Source::Null],
    }
}

/// The size that an allocation always has: a product of literals and
/// `sizeof` that is not 0 and that C computes as written. `Expr::constant`
/// folds without C's conversions, so nothing else counts as known.
pub fn fixed_size(program: &Program, bytes: &Expr) -> Option<i128> {
    fn plain(expr: &Expr) -> bool {
        match &in_size(expr).kind {
            ExprKind::Int(value) => *value >= 0,
            ExprKind::SizeOf(_) => true,
            ExprKind::Binary(BinOp::Mul, left, right) => {
                matches!(expr.ty.int(), Some((64, _))) && plain(left) && plain(right)
            }
            _ => false,
        }
    }
    let size = bytes.constant(program)?;
    (plain(bytes) && (1..=i128::from(i64::MAX)).contains(&size)).then_some(size)
}

/// The expression without the conversions to 64 bits around it, which
/// keep the value of a size. A conversion to fewer bits cuts it.
pub fn in_size(expr: &Expr) -> &Expr {
    match &expr.kind {
        ExprKind::Cast(inner) if matches!(expr.ty.int(), Some((64, _))) => in_size(inner),
        _ => expr,
    }
}
