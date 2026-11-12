//! The analysis: what the whole program does with each pointer, variable and
//! global. The rules read these facts and never look beyond the expression
//! they translate, so a question about the rest of the program belongs here.
//!
//! A *slot* is a place that holds a pointer: a variable, a record's field or
//! a function's result. Every assignment, argument and `return` is a *flow*
//! from a pointer's source into a slot, and the facts spread along the flows
//! until nothing changes.

use crate::c::{BinOp, Expr, ExprKind, FuncId, Program, RecordId, Stmt, Type, UnOp, VarId};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Slot {
    Var(VarId),
    Field(RecordId, usize),
    Return(FuncId),
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

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Facts {
    pointers: HashMap<Slot, Pointer>,
    mutated: HashSet<VarId>,
    through: HashSet<VarId>,
    access: Vec<Access>,
    /// False for the facts before the first round, which know nothing.
    settled: bool,
}

/// Library functions that store through a pointer argument, with the first
/// such argument and whether all later ones are stored through too.
const STORES: [(&str, usize, bool); 16] = [
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
];

/// The library functions whose result is fresh memory that `free` takes.
pub const ALLOCATORS: [&str; 2] = ["malloc", "calloc"];

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

    /// How the function, with everything it calls, uses the globals that
    /// the program changes.
    pub fn access(&self, function: FuncId) -> Access {
        self.access.get(function).copied().unwrap_or_default()
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
            Source::Slot(slot) => !self.pointer(*slot).nullable,
            Source::Fresh { .. } | Source::Place { .. } | Source::Literal => true,
            Source::Null | Source::Unknown(_) => false,
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
                (_, true) => sources(inner),
                (_, false) => vec![Source::Unknown("a pointer made from a number")],
            },
            ExprKind::AddrOf(place) => vec![Source::Place {
                root: self.root(program, place),
                array: false,
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
            ExprKind::Index(..) | ExprKind::Deref(_) => {
                vec![Source::Unknown(
                    "a pointer stored in an array or behind a pointer",
                )]
            }
            ExprKind::Function(_) => vec![Source::Unknown("a pointer to a function")],
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

/// The slot of a place that holds a pointer, if the analysis follows it.
pub fn place_slot(place: &Expr) -> Option<Slot> {
    if !place.ty.is_pointer() {
        return None;
    }
    match place.kind {
        ExprKind::Var(var) => Some(Slot::Var(var)),
        ExprKind::Member { record, field, .. } => Some(Slot::Field(record, field)),
        _ => None,
    }
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
    pointers: HashMap<Slot, Pointer>,
    mutated: HashSet<VarId>,
    through: HashSet<VarId>,
    /// The functions that change a global themselves.
    writers: HashSet<FuncId>,
    /// The globals each function names.
    named: Vec<HashSet<VarId>>,
    calls: Vec<(FuncId, FuncId)>,
}

impl<'a> Walk<'a> {
    fn new(program: &'a Program, known: &'a Facts) -> Self {
        Walk {
            program,
            known,
            function: None,
            flows: Vec::new(),
            pointers: HashMap::new(),
            mutated: HashSet::new(),
            through: HashSet::new(),
            writers: HashSet::new(),
            named: vec![HashSet::new(); program.functions.len()],
            calls: Vec::new(),
        }
    }

    fn run(mut self) -> Facts {
        let program = self.program;
        // What a record or a global holds lives on after the function that
        // stored it, so a pointer there owns what it points to: Rust has no
        // borrow that a rule here could give it.
        for (id, record) in program.records.iter().enumerate() {
            for (index, field) in record.fields.iter().enumerate() {
                if field.ty.is_pointer() {
                    self.seed(Slot::Field(id, index)).owned = true;
                }
            }
        }
        for &global in &program.globals {
            let var = &program.vars[global];
            if var.ty.is_pointer() {
                self.seed(Slot::Var(global)).owned = true;
            }
            let slot = var.ty.is_pointer().then_some(Slot::Var(global));
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
        // A function uses the globals at least as its callees do.
        loop {
            let mut changed = false;
            for &(caller, callee) in &self.calls {
                if access[caller] < access[callee] {
                    access[caller] = access[callee];
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        Facts {
            pointers: self.pointers,
            mutated: self.mutated,
            through: self.through,
            access,
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
                let slot = var.ty.is_pointer().then_some(Slot::Var(*id));
                match &var.init {
                    Some(init) => {
                        self.init(&var.ty, slot, init);
                        self.expr(init);
                    }
                    None if var.kind.is_global() => self.zero(&var.ty, slot),
                    None => {}
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
                self.flow_of(source, Slot::Var(*var), true);
                self.expr(source);
                self.stmts(then);
                self.stmts(other);
            }
            Stmt::LetSome {
                var,
                source,
                otherwise,
            } => {
                self.flow_of(source, Slot::Var(*var), true);
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
                        (Some(_), Some(slot)) => self.seed(slot).many = true,
                        (_, None) => {}
                    }
                }
            }
            ExprKind::IncDec { target, .. } => {
                self.write(target);
                if let Some(slot) = place_slot(target) {
                    self.seed(slot).many = true;
                }
            }
            ExprKind::Index(base, index) => {
                if index.constant(program) != Some(0) {
                    self.many(base);
                }
            }
            ExprKind::Binary(BinOp::Add | BinOp::Sub, left, right) => {
                for side in [left, right] {
                    if side.ty.is_pointer() {
                        self.many(side);
                    }
                }
            }
            ExprKind::Call(callee, args) => {
                if let Some(name) = callee.function() {
                    self.call(name, args);
                }
            }
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
        if let ("free", Some(freed)) = (name, args.first()) {
            for source in self.sources(freed) {
                if let Source::Slot(slot) = source {
                    self.seed(slot).owned = true;
                }
            }
            // What a field owned is taken out of its record.
            if matches!(freed.uncast().kind, ExprKind::Member { .. }) {
                self.write(freed.uncast());
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
                    let slot = field.ty.is_pointer().then_some(Slot::Field(*record, index));
                    match items.get(index) {
                        Some(item) => self.init(&field.ty, slot, item),
                        None => self.zero(&field.ty, slot),
                    }
                }
            }
            (Type::Array(element, length), ExprKind::InitList(items)) => {
                for item in items {
                    self.init(element, None, item);
                }
                if length.is_none_or(|length| (items.len() as u64) < length) {
                    self.zero(element, None);
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
                    let slot = field.ty.is_pointer().then_some(Slot::Field(*record, index));
                    self.zero(&field.ty, slot);
                }
            }
            Type::Array(element, _) => self.zero(element, None),
            _ => {}
        }
    }

    fn sources(&self, pointer: &Expr) -> Vec<Source> {
        self.known.sources(self.program, pointer)
    }

    fn seed(&mut self, slot: Slot) -> &mut Pointer {
        self.pointers.entry(slot).or_default()
    }

    /// Records that the pointer's value goes into the slot.
    fn flow(&mut self, pointer: &Expr, to: Slot) {
        self.flow_of(pointer, to, false);
    }

    /// Records that the pointer's value goes into the slot; `set` says that
    /// a test has found it not to be NULL.
    fn flow_of(&mut self, pointer: &Expr, to: Slot, set: bool) {
        let held = pointer.uncast();
        let holder = match held.kind {
            ExprKind::Member { .. } => Some(self.known.root(self.program, held)),
            ExprKind::Var(var) if self.program.vars[var].kind.is_global() => Some(Root::Var(var)),
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
    }

    /// Records a store to the place.
    fn write(&mut self, place: &Expr) {
        let root = self.known.root(self.program, place);
        self.write_root(&root, self.function);
        // Rust reaches what a field points to through the record, so a
        // store through the field needs a writable record.
        if let Some(pointer) = dereferenced(place)
            && matches!(pointer.uncast().kind, ExprKind::Member { .. })
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
        loop {
            let before = (
                self.pointers.clone(),
                self.mutated.len(),
                self.through.len(),
                self.writers.len(),
            );
            for flow in &flows {
                let mut to = *self.seed(flow.to);
                match &flow.from {
                    Source::Slot(slot) => {
                        let mut from = *self.seed(*slot);
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
                        from.owned |= to.owned;
                        // A result has no other owner, and what a record,
                        // a result or a global holds outlives a local.
                        to.owned |= from.owned
                            && (matches!(slot, Slot::Return(_)) || self.outlives_locals(flow.to));
                        to.nullable |= from.nullable && !flow.set;
                        *self.seed(*slot) = from;
                    }
                    Source::Null => to.nullable |= !flow.set,
                    Source::Fresh { many } => {
                        to.owned = true;
                        to.many |= many;
                    }
                    Source::Place { root, array } => {
                        to.many |= array;
                        if to.written {
                            self.write_root(root, flow.function);
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
                    Source::Unknown(what) => to.conflict = Some(what),
                }
                *self.seed(flow.to) = to;
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

    fn outlives_locals(&self, slot: Slot) -> bool {
        match slot {
            Slot::Var(var) => self.program.vars[var].kind.is_global(),
            Slot::Field(..) | Slot::Return(_) => true,
        }
    }
}
