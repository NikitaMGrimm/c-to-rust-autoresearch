//! Generates a private set of small C programs that combine constructs, from
//! the secret seed, so that a translation rule which handles each construct
//! alone but not their combination shows up. Wrong translations have mostly
//! been found where constructs meet: a pointer field moved out and read again,
//! a `free` in a branch before a later use, pointer arithmetic beside
//! indexing, a `goto` into a block.
//!
//! The generator keeps an abstract state while it writes statements: a range
//! for every integer, the targets of every pointer, and whether heap memory is
//! live, so a program is free of undefined behaviour by construction on every
//! input in range. The reference checks are the last filter. Candidate `i`
//! depends only on the seed and `i`, and the set is the first N candidates
//! that pass, so a larger N keeps a smaller set as its prefix. Only counts are
//! printed.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use bench::Rng;
use eval::Input;

const USAGE: &str = "usage: generate PRIVATE_DIR [--count N] [--jobs N]\n\
     Writes N programs derived from PRIVATE_DIR/seed to PRIVATE_DIR/generated.";
const COUNT: usize = 300;
/// Candidates tried per program asked for, before giving up.
const TRIES: usize = 4;
/// Inputs are numbers in -BOUND..=BOUND.
const BOUND: i128 = 999;
const LINES: (usize, usize) = (15, 80);
/// Nodes a list may hold; sums of node values stay within `int` below it.
const LIST_CAP: i128 = 48;
/// Loop iterations, nested ones multiplied, that a statement may run in.
const ITERATIONS: i128 = 400;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Family {
    Integers,
    Control,
    Arrays,
    Pointers,
    Structs,
    Heap,
    Functions,
    Strings,
    Goto,
    /// Pointers that share an object where a translation would move it.
    Sharing,
}

const FAMILIES: [(Family, u64); 10] = [
    (Family::Integers, 4),
    (Family::Control, 4),
    (Family::Arrays, 4),
    (Family::Pointers, 4),
    (Family::Structs, 4),
    (Family::Heap, 4),
    (Family::Functions, 3),
    (Family::Strings, 3),
    (Family::Goto, 3),
    (Family::Sharing, 6),
];

const WORDS: [&str; 40] = [
    "total", "count", "value", "step", "limit", "delta", "score", "width", "height", "level",
    "offset", "mark", "base", "gain", "price", "weight", "depth", "span", "rate", "bonus", "carry",
    "speed", "cost", "size", "low", "high", "sum", "acc", "key", "seed", "state", "phase", "turn",
    "gap", "peak", "result", "amount", "budget", "stock", "ticks",
];
const TEXTS: [&str; 12] = [
    "alpha", "river", "stone", "maple", "cloud", "ember", "lemon", "orbit", "quiet", "tiger",
    "delta", "frost",
];

struct Args {
    private: PathBuf,
    count: usize,
    jobs: usize,
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Option<Args> {
    let (mut positional, mut count, mut jobs) = (Vec::new(), COUNT, 2);
    let mut rest = args.into_iter();
    while let Some(arg) = rest.next() {
        let mut number = || rest.next()?.parse().ok().filter(|&n: &usize| n > 0);
        match arg.as_str() {
            "--count" => count = number()?,
            "--jobs" => jobs = number()?,
            _ if arg.starts_with('-') => return None,
            _ => positional.push(arg),
        }
    }
    let [private] = <[String; 1]>::try_from(positional).ok()?;
    Some(Args {
        private: private.into(),
        count,
        jobs,
    })
}

fn main() -> ExitCode {
    let Some(args) = parse_args(std::env::args().skip(1)) else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    match generate(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err}");
            ExitCode::FAILURE
        }
    }
}

struct Candidate {
    source: String,
    inputs: Vec<Input>,
    families: BTreeSet<Family>,
    /// The shapes of shared pointers and conversions it contains, 1 to 8.
    shapes: BTreeSet<u8>,
}

/// A candidate's valid inputs and the reasons its other inputs failed, or
/// why the candidate failed.
type Checked = Result<(Vec<Input>, Vec<&'static str>), &'static str>;

fn generate(args: &Args) -> Result<(), String> {
    let seed = bench::seed(&args.private)?;
    let scratch = eval::Scratch::new("generate")?;
    let limit = args.count * TRIES;
    let next = AtomicUsize::new(0);
    let cutoff = AtomicUsize::new(usize::MAX);
    let checked: Mutex<BTreeMap<usize, (Candidate, Checked)>> = Mutex::new(BTreeMap::new());
    std::thread::scope(|scope| {
        for _ in 0..args.jobs {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    if index >= limit || index > cutoff.load(Ordering::Relaxed) {
                        break;
                    }
                    let candidate = candidate(seed, index);
                    let work = scratch.join(index.to_string());
                    let result = check(&candidate, &work);
                    let _ = std::fs::remove_dir_all(&work);
                    let mut checked = checked.lock().unwrap();
                    checked.insert(index, (candidate, result));
                    if let Some(last) = nth_kept(&checked, args.count) {
                        cutoff.fetch_min(last, Ordering::Relaxed);
                    }
                }
            });
        }
    });
    let checked = checked.into_inner().unwrap();
    let Some(last) = nth_kept(&checked, args.count) else {
        return Err(format!(
            "only {} of {limit} candidates passed",
            checked
                .values()
                .filter(|(_, result)| result.is_ok())
                .count()
        ));
    };
    let out = args.private.join("generated");
    clear(&out)?;
    let mut rejected: BTreeMap<&str, usize> = BTreeMap::new();
    let mut dropped: BTreeMap<&str, usize> = BTreeMap::new();
    let mut families: BTreeMap<Family, usize> = BTreeMap::new();
    let mut shapes: BTreeMap<u8, usize> = BTreeMap::new();
    let mut kept = 0;
    // Only the prefix up to the last kept candidate counts, whatever the
    // workers checked beyond it.
    for (index, (candidate, result)) in checked.range(..=last) {
        match result {
            Ok((inputs, failed)) => {
                kept += 1;
                write(&out, &format!("gen_{kept:04}"), *index, candidate, inputs)?;
                for reason in failed {
                    *dropped.entry(reason).or_default() += 1;
                }
                for family in &candidate.families {
                    *families.entry(*family).or_default() += 1;
                }
                for shape in &candidate.shapes {
                    *shapes.entry(*shape).or_default() += 1;
                }
            }
            Err(reason) => *rejected.entry(reason).or_default() += 1,
        }
    }
    println!("tried {}, kept {kept}", last + 1);
    println!("rejected: {rejected:?}");
    println!("inputs dropped from kept programs: {dropped:?}");
    println!("programs per family: {families:?}");
    println!("programs per shape: {shapes:?}");
    Ok(())
}

/// The index of the `n`th candidate that passed, once every candidate before
/// it has been checked.
fn nth_kept(checked: &BTreeMap<usize, (Candidate, Checked)>, n: usize) -> Option<usize> {
    let mut kept = 0;
    for (expected, (&index, (_, result))) in checked.iter().enumerate() {
        if index != expected {
            return None;
        }
        kept += usize::from(result.is_ok());
        if kept == n {
            return Some(index);
        }
    }
    None
}

/// Keeps the inputs on which the candidate is a valid C reference, checked as
/// the benchmarks are; it needs two that print differently.
fn check(candidate: &Candidate, work: &Path) -> Checked {
    std::fs::create_dir_all(work).map_err(|_| "scratch")?;
    let c_file = work.join("program.c");
    std::fs::write(&c_file, &candidate.source).map_err(|_| "scratch")?;
    let references = eval::References::build(&c_file, &[], work).map_err(|_| "does_not_build")?;
    let (mut valid, mut failed, mut outputs) = (Vec::new(), Vec::new(), BTreeSet::new());
    for input in &candidate.inputs {
        match references.check(input) {
            Ok(run) => {
                outputs.insert(run.stdout);
                valid.push(input.clone());
            }
            Err(reason) => failed.push(reason),
        }
    }
    if valid.len() < 2 {
        return Err(failed.first().copied().unwrap_or("too_few_inputs"));
    }
    if outputs.len() < 2 {
        return Err("same_output");
    }
    Ok((valid, failed))
}

fn clear(dir: &Path) -> Result<(), String> {
    if dir.exists() {
        std::fs::remove_dir_all(dir)
            .map_err(|err| format!("cannot clear {}: {err}", dir.display()))?;
    }
    std::fs::create_dir_all(dir).map_err(|err| format!("cannot create {}: {err}", dir.display()))
}

fn write(
    dir: &Path,
    name: &str,
    index: usize,
    candidate: &Candidate,
    inputs: &[Input],
) -> Result<(), String> {
    let inputs: Vec<_> = inputs
        .iter()
        .map(|input| {
            serde_json::json!({
                "name": input.name,
                "args": input.args,
                "stdin": String::from_utf8_lossy(&input.stdin),
            })
        })
        .collect();
    let json = serde_json::json!({
        "source": format!("generated candidate {index}"),
        "inputs": inputs,
    });
    let base = dir.join(name);
    std::fs::write(base.with_extension("c"), &candidate.source)
        .and_then(|()| std::fs::write(base.with_extension("json"), format!("{json:#}\n")))
        .map_err(|err| format!("cannot write {}: {err}", base.display()))
}

/// Candidate `index`, which depends only on the seed and the index.
fn candidate(seed: u64, index: usize) -> Candidate {
    let mut rng = Rng(bench::mix(seed ^ bench::fnv(&format!("generate/{index}"))));
    let mut budget = 20 + rng.below(56) as usize;
    loop {
        let mut generator = Gen::new(&mut rng);
        let (source, families, shapes) = generator.program(budget);
        let count = generator.inputs;
        let lines = source.lines().count();
        if (LINES.0..=LINES.1).contains(&lines) {
            let inputs = inputs(&mut rng, count);
            return Candidate {
                source,
                inputs,
                families,
                shapes,
            };
        }
        budget = budget.saturating_sub(5).max(20);
    }
}

fn inputs(rng: &mut Rng, count: usize) -> Vec<Input> {
    let kinds = ["zeros", "small", "negative", "bounds", "mixed"];
    let skip = rng.below(kinds.len() as u64) as usize;
    kinds
        .iter()
        .enumerate()
        .filter(|&(i, _)| i != skip)
        .map(|(_, &kind)| {
            let numbers: Vec<String> = (0..count)
                .map(|_| {
                    let n: i128 = match kind {
                        "zeros" => 0,
                        "small" => 1 + rng.below(15) as i128,
                        "negative" => -1 - rng.below(60) as i128,
                        "bounds" => {
                            let n = BOUND - rng.below(10) as i128;
                            if rng.below(2) == 0 { n } else { -n }
                        }
                        _ => rng.below(2 * BOUND as u64 + 1) as i128 - BOUND,
                    };
                    n.to_string()
                })
                .collect();
            Input {
                name: kind.to_owned(),
                args: Vec::new(),
                stdin: format!("{}\n", numbers.join(" ")).into_bytes(),
            }
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ty {
    Char,
    UChar,
    Short,
    Int,
    UInt,
    Long,
    ULong,
}

impl Ty {
    fn name(self) -> &'static str {
        match self {
            Ty::Char => "char",
            Ty::UChar => "unsigned char",
            Ty::Short => "short",
            Ty::Int => "int",
            Ty::UInt => "unsigned",
            Ty::Long => "long",
            Ty::ULong => "unsigned long",
        }
    }

    fn range(self) -> (i128, i128) {
        match self {
            Ty::Char => (-128, 127),
            Ty::UChar => (0, 255),
            Ty::Short => (-32768, 32767),
            Ty::Int => (i32::MIN.into(), i32::MAX.into()),
            Ty::UInt => (0, u32::MAX.into()),
            Ty::Long => (i64::MIN.into(), i64::MAX.into()),
            Ty::ULong => (0, u64::MAX.into()),
        }
    }

    fn unsigned(self) -> bool {
        matches!(self, Ty::UChar | Ty::UInt | Ty::ULong)
    }

    fn promoted(self) -> Ty {
        match self {
            Ty::Char | Ty::UChar | Ty::Short => Ty::Int,
            ty => ty,
        }
    }

    fn format(self) -> &'static str {
        match self.promoted() {
            Ty::Int => "%d",
            Ty::UInt => "%u",
            Ty::Long => "%ld",
            _ => "%lu",
        }
    }
}

/// The type of a binary operation after the usual arithmetic conversions, for
/// 32-bit `int` and 64-bit `long`.
fn common(a: Ty, b: Ty) -> Ty {
    let (a, b) = (a.promoted(), b.promoted());
    if a == Ty::ULong || b == Ty::ULong {
        Ty::ULong
    } else if a == Ty::Long || b == Ty::Long {
        Ty::Long
    } else if a == Ty::UInt || b == Ty::UInt {
        Ty::UInt
    } else {
        Ty::Int
    }
}

/// An expression with its type and the range its value lies in.
#[derive(Debug, Clone)]
struct E {
    text: String,
    ty: Ty,
    lo: i128,
    hi: i128,
}

impl E {
    fn new(text: impl Into<String>, ty: Ty, (lo, hi): (i128, i128)) -> E {
        E {
            text: text.into(),
            ty,
            lo,
            hi,
        }
    }

    fn lit(value: i128) -> E {
        let text = if value < 0 {
            format!("({value})")
        } else {
            value.to_string()
        };
        E::new(text, Ty::Int, (value, value))
    }

    /// The range after conversion to `ty`. Conversions to a signed type
    /// that do not fit happen only in operations that never need them.
    fn range_as(&self, ty: Ty) -> (i128, i128) {
        let (min, max) = ty.range();
        if self.lo >= min && self.hi <= max {
            (self.lo, self.hi)
        } else {
            (min, max)
        }
    }
}

/// `text` without parentheses that enclose all of it.
fn bare(text: &str) -> &str {
    let Some(inner) = text.strip_prefix('(').and_then(|t| t.strip_suffix(')')) else {
        return text;
    };
    let mut depth = 0;
    for c in inner.chars() {
        depth += match c {
            '(' => 1,
            ')' => -1,
            _ => 0,
        };
        if depth < 0 {
            return text;
        }
    }
    inner
}

/// `a op b`, or `None` where the result might overflow a signed type or a
/// divisor might be zero.
fn binary(op: &str, a: &E, b: &E) -> Option<E> {
    let ty = common(a.ty, b.ty);
    let (min, max) = ty.range();
    let ((al, ah), (bl, bh)) = (a.range_as(ty), b.range_as(ty));
    let corners = |f: &dyn Fn(i128, i128) -> Option<i128>| {
        let values: Option<Vec<i128>> = [(al, bl), (al, bh), (ah, bl), (ah, bh)]
            .iter()
            .map(|&(x, y)| f(x, y))
            .collect();
        values.map(|v| (*v.iter().min().unwrap(), *v.iter().max().unwrap()))
    };
    let range = match op {
        "+" => Some((al + bl, ah + bh)),
        "-" => Some((al - bh, ah - bl)),
        "*" => corners(&|x, y| x.checked_mul(y)),
        // A divisor of -1 could overflow, so a negative one is -2 or less.
        "/" if bl >= 1 || (bh <= -2 && !ty.unsigned()) => corners(&|x, y| Some(x / y)),
        "%" if bl >= 1 || (bh <= -2 && !ty.unsigned()) => {
            let most = bh.max(-bl);
            Some((
                if al < 0 { al.max(1 - most) } else { 0 },
                if ah > 0 { ah.min(most - 1) } else { 0 },
            ))
        }
        "&" | "|" | "^" if al >= 0 && bl >= 0 => {
            let mask = |x: i128| ((x as u128 + 1).next_power_of_two() - 1) as i128;
            Some(if op == "&" {
                (0, ah.min(bh))
            } else {
                (0, mask(ah.max(bh)))
            })
        }
        _ => return None,
    };
    let range = match range {
        Some((lo, hi)) if lo >= min && hi <= max => (lo, hi),
        _ if ty.unsigned() => (0, max),
        _ => return None,
    };
    Some(E::new(format!("({} {op} {})", a.text, b.text), ty, range))
}

fn shift(op: &str, a: &E, k: u32) -> Option<E> {
    let ty = a.ty.promoted();
    let (min, max) = ty.range();
    let (lo, hi) = a.range_as(ty);
    if lo < 0 {
        return None;
    }
    let range = if op == ">>" {
        (lo >> k, hi >> k)
    } else if hi << k <= max {
        (lo << k, hi << k)
    } else if ty.unsigned() {
        (min, max)
    } else {
        return None;
    };
    Some(E::new(format!("({} {op} {k})", a.text), ty, range))
}

/// A readable or writable object: for a write, the range a value must fit.
#[derive(Debug, Clone)]
struct Place {
    text: String,
    ty: Ty,
    lo: i128,
    hi: i128,
}

impl Place {
    fn new(text: String, ty: Ty, (lo, hi): (i128, i128)) -> Place {
        Place { text, ty, lo, hi }
    }

    fn int(text: String, home: i128) -> Place {
        Place::new(text, Ty::Int, (-home, home))
    }

    fn e(&self) -> E {
        E::new(self.text.clone(), self.ty, (self.lo, self.hi))
    }
}

#[derive(Debug, Clone)]
struct Var {
    name: String,
    ty: Ty,
    lo: i128,
    hi: i128,
    writable: bool,
    /// Declared at the top level of `main`, so it lives to its end.
    top: bool,
    decl: usize,
}

impl Var {
    /// A loop counter or other number that the program does not assign.
    fn counter(name: String, lo: i128, hi: i128) -> Var {
        Var {
            name,
            ty: Ty::Int,
            lo,
            hi,
            writable: false,
            top: false,
            decl: usize::MAX,
        }
    }
}

struct Array {
    name: String,
    dims: Vec<usize>,
    home: i128,
    decl: usize,
}

#[derive(Debug, Clone, PartialEq)]
enum Target {
    Vars(BTreeSet<usize>),
    /// Into `arrays[arr]`, at an offset in the range.
    Array(usize, usize, usize),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Owned {
    Live,
    MaybeNull,
    Null,
}

enum Field {
    Scalar(String, Ty, i128),
    Array(String, usize, i128),
    Nested(String, usize),
}

struct Record {
    tag: String,
    fields: Vec<Field>,
}

/// A scalar path in a record, such as `.lo` or `.inner.x`; `len` for an
/// array field, whose elements need an index after the path.
struct FieldPath {
    path: String,
    ty: Ty,
    home: i128,
    len: Option<usize>,
}

impl FieldPath {
    /// Unsigned fields take any value, which wraps.
    fn range(&self) -> (i128, i128) {
        if self.ty.unsigned() {
            self.ty.range()
        } else {
            (-self.home, self.home)
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum InstKind {
    Value,
    Array(usize),
    Pointer,
}

struct Inst {
    name: String,
    rec: usize,
    kind: InstKind,
    decl: usize,
}

/// The parts of the abstract state that differ between paths and are joined
/// where paths meet.
#[derive(Debug, Clone, Default)]
struct State {
    targets: Vec<Target>,
    /// Heap `int` arrays: whether live, and their length.
    buffers: Vec<(Owned, usize)>,
    /// Lists: the most nodes each may hold.
    lists: Vec<i128>,
    /// Aliases into lists: whether usable, and the lists they point into.
    aliases: Vec<(bool, BTreeSet<usize>)>,
    /// Strings: least and most length.
    strings: Vec<(usize, usize)>,
}

impl State {
    fn join(&mut self, other: &State) {
        for (a, b) in self.targets.iter_mut().zip(&other.targets) {
            match (a, b) {
                (Target::Vars(a), Target::Vars(b)) => a.extend(b),
                (Target::Array(_, lo, hi), Target::Array(_, olo, ohi)) => {
                    *lo = (*lo).min(*olo);
                    *hi = (*hi).max(*ohi);
                }
                _ => unreachable!("a pointer keeps its kind of target"),
            }
        }
        for (a, b) in self.buffers.iter_mut().zip(&other.buffers) {
            if a.0 != b.0 {
                a.0 = Owned::MaybeNull;
            }
            a.1 = a.1.min(b.1);
        }
        for (a, b) in self.lists.iter_mut().zip(&other.lists) {
            *a = (*a).max(*b);
        }
        for (a, b) in self.aliases.iter_mut().zip(&other.aliases) {
            a.0 &= b.0;
            a.1.extend(&b.1);
        }
        for (a, b) in self.strings.iter_mut().zip(&other.strings) {
            *a = (a.0.min(b.0), a.1.max(b.1));
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Loop {
    For,
    Other,
    Switch,
}

struct Cleanup {
    label: String,
    /// Objects declared from this one on are not set where the gotos jump.
    first: usize,
    gotos: usize,
}

struct Node {
    tag: String,
    val: String,
    next: String,
}

/// Writes one program while keeping the abstract state of its `main`.
struct Gen<'r> {
    rng: &'r mut Rng,
    families: Vec<Family>,
    used: BTreeSet<Family>,
    lines: Vec<String>,
    indent: usize,
    top: Vec<String>,
    helpers: Vec<String>,
    helper_names: BTreeMap<String, String>,
    names: BTreeSet<String>,
    vars: Vec<Var>,
    globals: Vec<Var>,
    arrays: Vec<Array>,
    pointers: Vec<(String, usize)>,
    buffers: Vec<(String, i128, usize)>,
    records: Vec<Record>,
    insts: Vec<Inst>,
    node: Option<Node>,
    lists: Vec<(String, usize)>,
    aliases: Vec<String>,
    strings: Vec<(String, usize, usize)>,
    state: State,
    temps: Vec<Place>,
    nest: usize,
    no_decl: bool,
    simple: usize,
    loops: Vec<Loop>,
    mult: i128,
    decls: usize,
    cleanup: Option<Cleanup>,
    /// States where a `break` leaves each enclosing `switch`.
    breaks: Vec<State>,
    inputs: usize,
    shapes: BTreeSet<u8>,
    /// A record holding a node pointer: tag, number field, pointer field.
    holder: Option<(String, String, String)>,
    /// A record holding a heap array: tag, length field, pointer field.
    vector: Option<(String, String, String)>,
}

type Generator<'r> = fn(&mut Gen<'r>) -> bool;

impl<'r> Gen<'r> {
    fn new(rng: &'r mut Rng) -> Self {
        Gen {
            rng,
            families: Vec::new(),
            used: BTreeSet::new(),
            lines: Vec::new(),
            indent: 1,
            top: Vec::new(),
            helpers: Vec::new(),
            helper_names: BTreeMap::new(),
            names: BTreeSet::new(),
            vars: Vec::new(),
            globals: Vec::new(),
            arrays: Vec::new(),
            pointers: Vec::new(),
            buffers: Vec::new(),
            records: Vec::new(),
            insts: Vec::new(),
            node: None,
            lists: Vec::new(),
            aliases: Vec::new(),
            strings: Vec::new(),
            state: State::default(),
            temps: Vec::new(),
            nest: 0,
            no_decl: false,
            simple: 0,
            loops: Vec::new(),
            mult: 1,
            decls: 0,
            cleanup: None,
            breaks: Vec::new(),
            inputs: 0,
            shapes: BTreeSet::new(),
            holder: None,
            vector: None,
        }
    }

    fn below(&mut self, n: usize) -> usize {
        self.rng.below(n.max(1) as u64) as usize
    }

    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }

    fn range(&mut self, lo: i128, hi: i128) -> i128 {
        lo + self.rng.below((hi - lo + 1) as u64) as i128
    }

    fn pick<T: Clone>(&mut self, items: &[T]) -> T {
        items[self.below(items.len())].clone()
    }

    /// An identifier that nothing in scope uses.
    fn fresh(&mut self, words: &[&str]) -> String {
        let word = self.pick(words);
        let name = (0..)
            .map(|n| match n {
                0 => word.to_owned(),
                n => format!("{word}{n}"),
            })
            .find(|name| !self.names.contains(name))
            .unwrap();
        self.names.insert(name.clone());
        name
    }

    fn word(&mut self) -> String {
        self.fresh(&WORDS)
    }

    fn decl(&mut self) -> usize {
        self.decls += 1;
        self.decls - 1
    }

    fn line(&mut self, text: impl AsRef<str>) {
        let line = format!("{}{}", "    ".repeat(self.indent), text.as_ref());
        self.lines.push(line);
    }

    fn label(&mut self, label: &str) {
        let line = format!("{}{label}:", "    ".repeat(self.indent - 1));
        self.lines.push(line);
    }

    fn open(&mut self, header: impl AsRef<str>) {
        self.line(format!("{} {{", header.as_ref()));
        self.indent += 1;
    }

    fn close(&mut self) {
        self.indent -= 1;
        self.line("}");
    }

    fn reopen(&mut self, header: &str) {
        self.indent -= 1;
        self.line(format!("}} {header} {{"));
        self.indent += 1;
    }

    /// Runs `body` in a nested scope: what it declares ends with it.
    fn scoped(&mut self, body: impl FnOnce(&mut Self)) {
        self.scope(true, body);
    }

    /// Like `scoped`, but for statements in no block of their own, whose
    /// names stay taken.
    fn scope(&mut self, block: bool, body: impl FnOnce(&mut Self)) {
        let (vars, no_decl) = (self.vars.len(), self.no_decl);
        let names = if block {
            self.names.clone()
        } else {
            BTreeSet::new()
        };
        self.nest += 1;
        self.no_decl = false;
        body(self);
        // Names declared in the scope are free again; a loop counter declared
        // outside it and also listed inside it keeps its name.
        for var in self.vars.split_off(vars) {
            if block && !names.contains(&var.name) {
                self.names.remove(&var.name);
            }
        }
        self.nest -= 1;
        self.no_decl = no_decl;
    }

    fn in_loop(&self) -> bool {
        self.loops.iter().any(|kind| *kind != Loop::Switch) || self.mult > 1
    }

    /// Whether a declaration of an object that lives to the end of `main` may
    /// go here.
    fn top_level(&self) -> bool {
        self.nest == 0 && self.simple == 0
    }

    fn mark(&mut self, family: Family) {
        self.used.insert(family);
    }

    // The program.

    fn program(&mut self, budget: usize) -> (String, BTreeSet<Family>, BTreeSet<u8>) {
        let mut chosen: Vec<Family> = Vec::new();
        // Some programs are a shape of shared pointers and little else: a
        // translator that refuses one construct refuses the whole program.
        let focused = self.chance(35);
        let (count, budget) = if focused {
            chosen.push(Family::Sharing);
            (1 + self.below(2), budget.min(30 + self.below(15)))
        } else {
            (2 + self.below(4), budget)
        };
        while chosen.len() < count {
            let total: u64 = FAMILIES.iter().map(|(_, w)| w).sum();
            let mut roll = self.rng.below(total);
            for (family, weight) in FAMILIES {
                if roll < weight {
                    if !chosen.contains(&family) {
                        chosen.push(family);
                    }
                    break;
                }
                roll -= weight;
            }
        }
        self.families = chosen.clone();
        self.read_inputs();
        for family in chosen {
            self.setup(family);
        }
        let mut statements = 0;
        while self.size() + 6 < budget && statements < 40 {
            self.statement();
            statements += 1;
            if self.chance(25) {
                self.print();
            }
        }
        self.finish();
        let mut source =
            String::from("#include <stdio.h>\n#include <stdlib.h>\n#include <string.h>\n");
        for part in self.top.iter().chain(&self.helpers) {
            source.push('\n');
            source.push_str(part);
        }
        source.push_str("\nint main(void)\n{\n");
        for line in &self.lines {
            source.push_str(line);
            source.push('\n');
        }
        source.push_str("}\n");
        let shapes = std::mem::take(&mut self.shapes);
        (source, std::mem::take(&mut self.used), shapes)
    }

    fn size(&self) -> usize {
        let parts: usize = self
            .top
            .iter()
            .chain(&self.helpers)
            .map(|part| part.lines().count() + 1)
            .sum();
        parts + self.lines.len() + 8
    }

    fn read_inputs(&mut self) {
        let count = 1 + self.below(3);
        let names: Vec<String> = (0..count)
            .map(|_| self.fresh(&["a", "b", "c", "x", "y", "n", "m", "first", "second"]))
            .collect();
        self.line(format!("int {};", names.join(", ")));
        if self.chance(50) {
            let formats = vec!["%d"; count].join(" ");
            let addresses: Vec<String> = names.iter().map(|n| format!("&{n}")).collect();
            self.line(format!(
                "if (scanf(\"{formats}\", {}) != {count})",
                addresses.join(", ")
            ));
            self.line("    return 1;");
        } else {
            for name in &names {
                self.line(format!("if (scanf(\"%d\", &{name}) != 1)"));
                self.line("    return 1;");
            }
        }
        let checks: Vec<String> = names
            .iter()
            .map(|n| format!("{n} < -{BOUND} || {n} > {BOUND}"))
            .collect();
        self.line(format!("if ({})", checks.join(" || ")));
        self.line("    return 2;");
        self.inputs = names.len();
        for name in names {
            self.new_var(name, Ty::Int, (-BOUND, BOUND), false);
        }
    }

    /// The first statement of a family: its data, where it has some.
    fn setup(&mut self, family: Family) {
        let done = match family {
            Family::Arrays => self.array_decl(),
            Family::Pointers => self.pointer_decl(),
            Family::Structs => self.struct_decl(),
            Family::Heap if self.chance(50) => self.buffer_decl(),
            Family::Heap => self.list_decl(),
            Family::Strings => self.string_decl(),
            _ => (0..4).any(|_| self.try_family(family)),
        };
        if done {
            self.mark(family);
        }
    }

    fn statement(&mut self) {
        if self.simple > 0 {
            self.simple_statement();
            return;
        }
        for _ in 0..8 {
            let family = self.pick(&self.families.clone());
            if self.try_family(family) {
                self.mark(family);
                return;
            }
        }
        self.simple_statement();
    }

    /// Runs one random generator of `family`; whether it wrote anything.
    fn try_family(&mut self, family: Family) -> bool {
        {
            let options: &[Generator<'r>] = match family {
                Family::Integers => &[
                    Self::int_decl,
                    Self::assign,
                    Self::compound,
                    Self::step,
                    Self::small_decl,
                    Self::compare_print,
                    Self::widen_pair,
                    Self::post_increment,
                    Self::short_circuit,
                    Self::chain,
                ],
                Family::Control => &[
                    Self::if_else,
                    Self::for_loop,
                    Self::while_loop,
                    Self::do_while,
                    Self::switch,
                    Self::jump,
                    Self::countdown,
                ],
                Family::Arrays => &[
                    Self::array_decl,
                    Self::array_fill,
                    Self::array_algorithm,
                    Self::array_reduce,
                    Self::array_call,
                    Self::array_print,
                ],
                Family::Pointers => &[
                    Self::pointer_decl,
                    Self::retarget,
                    Self::pointer_walk,
                    Self::swap,
                    Self::out_params,
                    Self::pointer_diff,
                    Self::assign_through,
                ],
                Family::Structs => &[
                    Self::struct_decl,
                    Self::struct_copy,
                    Self::struct_call,
                    Self::struct_call,
                    Self::assign_field,
                ],
                Family::Heap => &[
                    Self::buffer_decl,
                    Self::buffer_change,
                    Self::buffer_guard,
                    Self::list_decl,
                    Self::list_op,
                    Self::list_op,
                    Self::node_guard,
                    Self::list_split,
                ],
                Family::Functions => &[
                    Self::recursion,
                    Self::function_pointer,
                    Self::static_counter,
                    Self::global_note,
                    Self::clamp_call,
                ],
                Family::Strings => &[Self::string_decl, Self::string_op, Self::string_op],
                Family::Sharing => &[
                    Self::shape_field_copy,
                    Self::shape_spellings,
                    Self::shape_chosen_owner,
                    Self::shape_cursor,
                    Self::shape_nested,
                    Self::shape_inner_pointer,
                    Self::shape_swap_owners,
                ],
                Family::Goto => &[
                    Self::goto_out,
                    Self::nested_exit,
                    Self::goto_loop,
                    Self::goto_skip,
                ],
            };
            let generator = options[self.below(options.len())];
            generator(self)
        }
    }

    /// A statement that changes no pointer or heap state.
    fn simple_statement(&mut self) {
        let done = match self.below(4) {
            0 => self.compound(),
            1 => self.step(),
            _ => self.assign(),
        };
        if done {
            self.mark(Family::Integers);
        } else if !self.print() {
            self.line("printf(\"-\\n\");");
        }
    }

    /// A block of one to three statements in a new scope.
    fn body(&mut self) {
        self.scoped(|g| {
            for _ in 0..1 + g.below(3) {
                g.statement();
            }
        });
    }

    // Expressions.

    fn scalars(&self, writable: bool) -> Vec<Var> {
        self.vars
            .iter()
            .chain(&self.globals)
            .filter(|v| v.writable || !writable)
            .cloned()
            .collect()
    }

    /// An index in `0..len`.
    fn index(&mut self, len: usize) -> String {
        let n = len as i128;
        let fitting: Vec<String> = self
            .vars
            .iter()
            .filter(|v| v.lo >= 0 && v.hi < n)
            .map(|v| v.name.clone())
            .collect();
        match self.below(4) {
            0 | 1 if !fitting.is_empty() => self.pick(&fitting),
            0 => self.below(len).to_string(),
            _ => {
                let vars = self.scalars(false);
                let var = self.pick(&vars);
                if var.lo >= 0 {
                    format!("{} % {len}", var.name)
                } else {
                    format!("({} % {len} + {len}) % {len}", var.name)
                }
            }
        }
    }

    /// A random object to read or, with `write`, to assign.
    fn place(&mut self, write: bool) -> Option<Place> {
        let mut kinds = Vec::new();
        if !self.scalars(write).is_empty() {
            kinds.extend([0, 0, 0]);
        }
        if !self.arrays.is_empty() {
            kinds.extend([1, 1]);
        }
        if !self.insts.is_empty() {
            kinds.extend([2, 2]);
        }
        if !self.pointers.is_empty() {
            kinds.extend([3, 3]);
        }
        if self.state.buffers.iter().any(|b| b.0 == Owned::Live) {
            kinds.extend([4, 4]);
        }
        if !self.temps.is_empty() {
            kinds.extend([5, 5, 5]);
        }
        if !write && !self.strings.is_empty() {
            kinds.push(6);
        }
        if kinds.is_empty() {
            return None;
        }
        Some(match self.pick(&kinds) {
            0 => {
                let mut vars = self.scalars(write);
                if !write {
                    // Inputs are read more often, so output depends on them.
                    vars.extend(self.vars[..self.inputs].iter().cloned());
                }
                let v = self.pick(&vars);
                Place::new(v.name, v.ty, (v.lo, v.hi))
            }
            1 => {
                let a = self.below(self.arrays.len());
                self.array_place(a)
            }
            2 => {
                let i = self.below(self.insts.len());
                self.field_place(i)
            }
            3 => {
                let p = self.below(self.pointers.len());
                self.pointer_place(p, write)
            }
            4 => {
                let live: Vec<usize> = (0..self.buffers.len())
                    .filter(|&b| self.state.buffers[b].0 == Owned::Live)
                    .collect();
                let b = self.pick(&live);
                let len = self.state.buffers[b].1;
                let (name, home, _) = self.buffers[b].clone();
                Place {
                    text: format!("{name}[{}]", self.index(len)),
                    ty: Ty::Int,
                    lo: -home,
                    hi: home,
                }
            }
            5 => {
                let temps = self.temps.clone();
                self.pick(&temps)
            }
            _ => {
                let (name, cap, _) = self.pick(&self.strings.clone());
                // Every byte of a string buffer is set: the literal that
                // initialises it fills the rest with zeros.
                Place {
                    text: format!("{name}[{}]", self.index(cap)),
                    ty: Ty::Char,
                    lo: 0,
                    hi: 122,
                }
            }
        })
    }

    fn array_place(&mut self, a: usize) -> Place {
        let dims = self.arrays[a].dims.clone();
        let mut text = self.arrays[a].name.clone();
        for len in dims {
            text = format!("{text}[{}]", self.index(len));
        }
        let home = self.arrays[a].home;
        Place::int(text, home)
    }

    fn field_place(&mut self, i: usize) -> Place {
        let paths = self.paths(self.insts[i].rec);
        let path = &paths[self.below(paths.len())];
        let (name, kind) = (self.insts[i].name.clone(), self.insts[i].kind);
        let mut text = match kind {
            InstKind::Value => format!("{name}{}", path.path),
            InstKind::Array(len) => format!("{name}[{}]{}", self.index(len), path.path),
            InstKind::Pointer => format!("{name}->{}", &path.path[1..]),
        };
        if let Some(len) = path.len {
            text = format!("{text}[{}]", self.index(len));
        }
        let (lo, hi) = path.range();
        Place::new(text, path.ty, (lo, hi))
    }

    /// The object pointer `p` points to; a value written must fit every
    /// object it may point to.
    fn pointer_place(&mut self, p: usize, write: bool) -> Place {
        let name = self.pointers[p].0.clone();
        match self.state.targets[p].clone() {
            Target::Vars(targets) => {
                let los = targets.iter().map(|&t| self.vars[t].lo);
                let his = targets.iter().map(|&t| self.vars[t].hi);
                let (lo, hi) = if write {
                    (los.max().unwrap(), his.min().unwrap())
                } else {
                    (los.min().unwrap(), his.max().unwrap())
                };
                Place::new(format!("*{name}"), Ty::Int, (lo, hi))
            }
            Target::Array(a, lo, hi) => {
                let len = self.arrays[a].dims[0] as i128;
                let k = self.range(-(lo as i128), len - 1 - hi as i128);
                let text = match (k, self.below(3)) {
                    (0, _) => format!("*{name}"),
                    (k, 0) if k > 0 => format!("{name}[{k}]"),
                    (k, _) if k > 0 => format!("*({name} + {k})"),
                    (k, _) => format!("*({name} - {})", -k),
                };
                let home = self.arrays[a].home;
                Place::int(text, home)
            }
        }
    }

    fn leaf(&mut self) -> E {
        if self.chance(25) {
            return E::lit(self.range(-9, 60));
        }
        match self.place(false) {
            Some(place) => place.e(),
            None => E::lit(self.range(1, 9)),
        }
    }

    fn expr(&mut self, depth: u32) -> E {
        if depth == 0 || self.chance(30) {
            return self.leaf();
        }
        let a = self.expr(depth - 1);
        match self.below(12) {
            0..=4 => {
                let op = self.pick(&["+", "-", "*", "+"]);
                let b = self.expr(depth - 1);
                self.arithmetic(op, a, b)
            }
            5 | 6 => {
                let op = if self.chance(50) { "/" } else { "%" };
                let d = self.divisor();
                let negative = E::new(format!("(-{})", d.text), d.ty, (-d.hi, -d.lo));
                let signed = !common(a.ty, d.ty).unsigned();
                if signed && self.chance(30) {
                    self.shapes.insert(8);
                    binary(op, &a, &negative).unwrap()
                } else {
                    binary(op, &a, &d).unwrap()
                }
            }
            7 => {
                let c = self.cond(depth - 1);
                let b = self.expr(depth - 1);
                let ty = common(a.ty, b.ty);
                let ((al, ah), (bl, bh)) = (a.range_as(ty), b.range_as(ty));
                E::new(
                    format!("({c} ? {} : {})", a.text, b.text),
                    ty,
                    (al.min(bl), ah.max(bh)),
                )
            }
            8 => {
                let to = self.pick(&[
                    Ty::UChar,
                    Ty::UInt,
                    Ty::Long,
                    Ty::ULong,
                    Ty::Short,
                    Ty::Char,
                ]);
                let (min, max) = to.range();
                if to.unsigned() || (a.lo >= min && a.hi <= max) {
                    let range = a.range_as(to);
                    E::new(format!("({}){}", to.name(), a.text), to, range)
                } else {
                    a
                }
            }
            9 if a.ty == Ty::UChar && self.chance(50) => {
                // Promoted to int before the shift.
                self.shapes.insert(8);
                shift("<<", &a, 1 + self.below(8) as u32).unwrap()
            }
            9 => {
                let u = if a.ty.unsigned() {
                    a
                } else {
                    let range = a.range_as(Ty::UInt);
                    E::new(format!("(unsigned){}", a.text), Ty::UInt, range)
                };
                match self.below(5) {
                    0 => shift(">>", &u, 1 + self.below(6) as u32).unwrap(),
                    1 => shift("<<", &u, 1 + self.below(4) as u32).unwrap_or(u),
                    2 => binary("&", &u, &E::lit(self.pick(&[15, 255, 1023]))).unwrap(),
                    3 => binary("^", &u, &E::lit(self.range(1, 255))).unwrap(),
                    _ => binary(
                        "*",
                        &u,
                        &E::new("2654435761u", Ty::UInt, (2654435761, 2654435761)),
                    )
                    .unwrap(),
                }
            }
            10 => {
                let b = self.expr(depth - 1);
                let op = self.pick(&["<", "<=", ">", ">=", "==", "!="]);
                E::new(format!("({} {op} {})", a.text, b.text), Ty::Int, (0, 1))
            }
            _ => {
                let (min, max) = a.ty.promoted().range();
                if a.ty.unsigned() || (-a.hi >= min && -a.lo <= max) {
                    let ty = a.ty.promoted();
                    let range = if ty.unsigned() {
                        (0, max)
                    } else {
                        (-a.hi, -a.lo)
                    };
                    E::new(format!("(-{})", a.text), ty, range)
                } else {
                    a
                }
            }
        }
    }

    /// `a op b`, with operands reduced until the result fits.
    fn arithmetic(&mut self, op: &str, a: E, b: E) -> E {
        if let Some(e) = binary(op, &a, &b) {
            return e;
        }
        let m = self.pick(&[10, 100, 1000]);
        let a = binary("%", &a, &E::lit(m)).unwrap();
        if let Some(e) = binary(op, &a, &b) {
            return e;
        }
        let b = binary("%", &b, &E::lit(m)).unwrap();
        binary(op, &a, &b).unwrap()
    }

    /// A divisor from 2 to 2k.
    fn divisor(&mut self) -> E {
        let k = self.pick(&[3, 5, 7, 10]);
        let e = self.expr(1);
        let rem = binary("%", &e, &E::lit(k)).unwrap();
        binary("+", &rem, &E::lit(k + 1)).unwrap()
    }

    fn cond(&mut self, depth: u32) -> String {
        let special = self.below(8);
        if special == 0
            && let Some(p) = (0..self.pointers.len())
                .find(|&p| matches!(self.state.targets[p], Target::Array(..)))
        {
            let Target::Array(a, ..) = self.state.targets[p] else {
                unreachable!()
            };
            let len = self.arrays[a].dims[0];
            let op = self.pick(&["<", ">=", "!=", "=="]);
            let k = self.below(len);
            return format!("{} {op} {} + {k}", self.pointers[p].0, self.arrays[a].name);
        }
        if special == 1 && !self.strings.is_empty() {
            let (a, ..) = self.pick(&self.strings.clone());
            let other = if self.chance(50) {
                self.pick(&self.strings.clone()).0
            } else {
                format!("\"{}\"", self.pick(&TEXTS))
            };
            let op = self.pick(&["<", ">", "==", "!="]);
            self.mark(Family::Strings);
            return format!("strcmp({a}, {other}) {op} 0");
        }
        let a = self.expr(depth.min(1));
        let mut b = self.expr(depth.min(1));
        if a.lo == a.hi && b.lo == b.hi {
            // Two constants would make one branch dead.
            b = self.place(false).map_or(b, |place| place.e());
        }
        let op = self.pick(&["<", "<=", ">", ">=", "==", "!="]);
        let compare = format!("{} {op} {}", a.text, b.text);
        if depth > 0 && self.chance(20) {
            let other = self.cond(0);
            let join = self.pick(&["&&", "||"]);
            format!("{compare} {join} {other}")
        } else {
            compare
        }
    }

    /// `e` converted to fit a place of type `ty` with range `lo..=hi`.
    fn fit(&mut self, e: E, ty: Ty, lo: i128, hi: i128) -> String {
        if ty.unsigned() || (e.lo >= lo && e.hi <= hi) {
            return bare(&e.text).to_owned();
        }
        let m = hi.min(-lo) + 1;
        let reduced = binary("%", &e, &E::lit(m)).unwrap();
        if matches!(ty, Ty::Char | Ty::Short) && self.chance(50) {
            format!("({}){}", ty.name(), reduced.text)
        } else {
            bare(&reduced.text).to_owned()
        }
    }

    fn fit_place(&mut self, e: E, place: &Place) -> String {
        self.fit(e, place.ty, place.lo, place.hi)
    }

    // Family 1: integers and conversions.

    fn new_var(&mut self, name: String, ty: Ty, (lo, hi): (i128, i128), writable: bool) {
        let decl = self.decl();
        let top = self.nest == 0;
        self.vars.push(Var {
            name,
            ty,
            lo,
            hi,
            writable,
            top,
            decl,
        });
    }

    fn int_decl(&mut self) -> bool {
        if self.no_decl {
            return false;
        }
        let ty = self.pick(&[
            Ty::Int,
            Ty::Int,
            Ty::Int,
            Ty::Int,
            Ty::UInt,
            Ty::Long,
            Ty::Char,
            Ty::UChar,
            Ty::Short,
            Ty::ULong,
        ]);
        let (lo, hi) = match ty {
            Ty::Int => {
                let r = self.pick(&[999, 9999, 99999]);
                (-r, r)
            }
            Ty::Long => (-1_000_000_000_000, 1_000_000_000_000),
            ty => ty.range(),
        };
        let e = self.expr(2);
        let value = self.fit(e, ty, lo, hi);
        let name = self.word();
        self.line(format!("{} {name} = {value};", ty.name()));
        self.new_var(name, ty, (lo, hi), true);
        true
    }

    /// A constant small number, for indices and loop bounds.
    fn small_decl(&mut self) -> bool {
        if self.no_decl {
            return false;
        }
        let k = self.range(2, 9);
        let e = self.expr(1);
        let reduced = binary("%", &e, &E::lit(k)).unwrap();
        let (text, range) = if reduced.lo >= 0 {
            (bare(&reduced.text).to_owned(), (reduced.lo, reduced.hi))
        } else {
            let shifted = binary("+", &reduced, &E::lit(k)).unwrap();
            (format!("{} % {k}", shifted.text), (0, k - 1))
        };
        let name = self.fresh(&["idx", "slot", "pick", "pos", "nth", "part"]);
        self.line(format!("int {name} = {text};"));
        self.new_var(name, Ty::Int, range, false);
        true
    }

    fn assign(&mut self) -> bool {
        let Some(place) = self.place(true) else {
            return false;
        };
        self.write(place);
        true
    }

    fn compound(&mut self) -> bool {
        let Some(place) = self.place(true) else {
            return false;
        };
        if place.ty.unsigned() {
            let line = match self.below(4) {
                0 => format!("{} <<= {};", place.text, 1 + self.below(3)),
                1 => format!("{} >>= {};", place.text, 1 + self.below(3)),
                _ => {
                    let op = self.pick(&["+=", "-=", "*=", "^=", "|="]);
                    let e = self.expr(1);
                    format!("{} {op} {};", place.text, bare(&e.text))
                }
            };
            self.line(line);
            return true;
        }
        if !matches!(place.ty, Ty::Int | Ty::Long | Ty::Short) {
            return false;
        }
        let op = self.pick(&["+", "-", "*"]);
        let e = self.expr(1);
        let e = if op == "*" {
            binary("%", &e, &E::lit(10)).unwrap()
        } else {
            e
        };
        let Some(result) = binary(op, &place.e(), &e) else {
            return false;
        };
        // The result converts back to the place's type, which must hold it.
        let (min, max) = place.ty.range();
        if result.lo < min || result.hi > max {
            return false;
        }
        if place.ty == Ty::Short {
            self.shapes.insert(8);
        }
        self.line(format!("{} {op}= {};", place.text, bare(&e.text)));
        if result.lo < place.lo || result.hi > place.hi {
            self.line(format!(
                "{} %= {};",
                place.text,
                place.hi.min(-place.lo) + 1
            ));
        }
        true
    }

    fn step(&mut self) -> bool {
        let Some(place) = self.place(true) else {
            return false;
        };
        if place.ty.unsigned() {
            let op = self.pick(&["++", "--"]);
            self.line(format!("{op}{};", place.text));
        } else if self.chance(50) {
            self.line(format!("if ({} < {})", place.text, place.hi));
            self.line(format!("    ++{};", place.text));
        } else {
            self.line(format!("if ({} > {})", place.text, place.lo));
            self.line(format!("    --{};", place.text));
        }
        true
    }

    fn print(&mut self) -> bool {
        if self.mult > 8 {
            return false;
        }
        let count = 1 + self.below(3);
        let places: Vec<Place> = (0..count).filter_map(|_| self.place(false)).collect();
        if places.is_empty() {
            return false;
        }
        let word = self.pick(&WORDS);
        let formats: Vec<&str> = places.iter().map(|p| p.ty.format()).collect();
        let texts: Vec<&str> = places.iter().map(|p| p.text.as_str()).collect();
        self.line(format!(
            "printf(\"{word}: {}\\n\", {});",
            formats.join(" "),
            texts.join(", ")
        ));
        true
    }

    // Family 2: control flow.

    fn if_else(&mut self) -> bool {
        if self.nest >= 3 {
            return false;
        }
        let c = self.cond(1);
        let before = self.state.clone();
        self.open(format!("if ({c})"));
        self.body();
        if self.chance(60) {
            let then = std::mem::replace(&mut self.state, before);
            if self.chance(30) && self.nest < 2 {
                let c = self.cond(0);
                let before = self.state.clone();
                self.reopen(&format!("else if ({c})"));
                self.body();
                self.state.join(&before);
            }
            self.reopen("else");
            self.body();
            self.state.join(&then);
        } else {
            self.state.join(&before);
        }
        self.close();
        true
    }

    /// A loop bound: its text and the most it can be.
    fn bound(&mut self) -> (String, i128) {
        let small: Vec<Var> = self
            .vars
            .iter()
            .filter(|v| v.hi >= 1 && v.hi <= 12)
            .cloned()
            .collect();
        match self.below(3) {
            0 if !small.is_empty() => {
                let v = self.pick(&small);
                (v.name, v.hi)
            }
            1 => {
                let k = self.range(3, 8);
                let e = self.expr(1);
                let reduced = binary("%", &e, &E::lit(k)).unwrap();
                (reduced.text, reduced.hi.max(0))
            }
            _ => {
                let k = self.range(2, 7);
                (k.to_string(), k)
            }
        }
    }

    fn enter_loop(&mut self, kind: Loop, iterations: i128) -> Option<i128> {
        if self.nest >= 3 || self.mult * iterations.max(1) > ITERATIONS {
            return None;
        }
        self.loops.push(kind);
        let mult = self.mult;
        self.mult *= iterations.max(1);
        Some(mult)
    }

    fn leave_loop(&mut self, mult: i128) {
        self.loops.pop();
        self.mult = mult;
    }

    fn for_loop(&mut self) -> bool {
        let (bound, hi) = self.bound();
        let Some(mult) = self.enter_loop(Loop::For, hi) else {
            return false;
        };
        let i = self.fresh(&["i", "j", "k"]);
        self.open(format!("for (int {i} = 0; {i} < {}; {i}++)", bare(&bound)));
        self.scoped(|g| {
            g.vars.push(Var::counter(i.clone(), 0, (hi - 1).max(0)));
            g.body();
        });
        self.names.remove(&i);
        self.close();
        self.leave_loop(mult);
        true
    }

    fn while_loop(&mut self) -> bool {
        if self.no_decl {
            return false;
        }
        let (bound, hi) = self.bound();
        let Some(mult) = self.enter_loop(Loop::Other, hi) else {
            return false;
        };
        let k = self.fresh(&["left", "rounds", "todo", "remaining"]);
        self.line(format!("int {k} = {};", bare(&bound)));
        self.open(format!("while ({k} > 0)"));
        self.scoped(|g| {
            g.vars.push(Var::counter(k.clone(), 1, hi.max(1)));
            g.body();
        });
        self.line(format!("{k}--;"));
        self.close();
        self.leave_loop(mult);
        true
    }

    fn do_while(&mut self) -> bool {
        if self.no_decl {
            return false;
        }
        let (bound, hi) = self.bound();
        let Some(mult) = self.enter_loop(Loop::Other, hi) else {
            return false;
        };
        let t = self.fresh(&["tries", "pass", "turns"]);
        self.line(format!("int {t} = 0;"));
        self.open("do");
        self.scoped(|g| {
            g.vars.push(Var::counter(t.clone(), 0, (hi - 1).max(0)));
            g.body();
        });
        self.line(format!("{t}++;"));
        self.indent -= 1;
        self.line(format!("}} while ({t} < {});", bare(&bound)));
        self.leave_loop(mult);
        true
    }

    fn switch(&mut self) -> bool {
        if self.nest >= 3 {
            return false;
        }
        let k = self.range(3, 5);
        let e = self.expr(1);
        let selector = binary("%", &e, &E::lit(k)).unwrap();
        let mut values: Vec<i128> = (selector.lo..=selector.hi).collect();
        let mut cases = Vec::new();
        for _ in 0..2 + self.below(2) {
            if values.is_empty() {
                break;
            }
            cases.push(values.remove(self.below(values.len())));
        }
        cases.sort();
        self.open(format!("switch ({})", bare(&selector.text)));
        self.loops.push(Loop::Switch);
        self.breaks.push(self.state.clone());
        let before = self.state.clone();
        let mut acc = before.clone();
        let labels: Vec<String> = cases
            .iter()
            .map(|c| format!("case {c}"))
            .chain(["default".to_owned()])
            .collect();
        let last = labels.len() - 1;
        for (n, label) in labels.into_iter().enumerate() {
            self.label(&label);
            self.state = acc.clone();
            self.scoped(|g| {
                g.no_decl = true;
                for _ in 0..1 + g.below(2) {
                    g.statement();
                }
            });
            if n == last || self.chance(60) {
                self.line("break;");
            } else {
                self.line("/* fall through */");
            }
            acc.join(&self.state);
        }
        acc.join(&self.breaks.pop().unwrap());
        self.state = acc;
        self.loops.pop();
        self.close();
        true
    }

    fn jump(&mut self) -> bool {
        let Some(&innermost) = self.loops.last() else {
            return false;
        };
        let continuable =
            self.loops.iter().rev().find(|&&kind| kind != Loop::Switch) == Some(&Loop::For);
        let percent = if innermost == Loop::Switch { 30 } else { 50 };
        let word = if continuable && self.chance(percent) {
            "continue"
        } else {
            "break"
        };
        if word == "break" && innermost == Loop::Switch {
            self.breaks.last_mut().unwrap().join(&self.state);
        }
        let c = self.cond(1);
        self.line(format!("if ({c})"));
        self.line(format!("    {word};"));
        true
    }

    // Family 3: arrays.

    fn array_decl(&mut self) -> bool {
        if !self.top_level() {
            return false;
        }
        let home = self.pick(&[99, 999, 9999]);
        let dims = if self.chance(25) {
            vec![self.range(2, 3) as usize, self.range(2, 4) as usize]
        } else {
            vec![self.range(3, 8) as usize]
        };
        let name = self.fresh(&[
            "vals", "data", "items", "grid", "table", "slots", "marks", "row",
        ]);
        let element = |g: &mut Self| -> String {
            if g.chance(25) {
                let e = g.expr(1);
                g.fit(e, Ty::Int, -home, home)
            } else {
                g.range(-20, 50).to_string()
            }
        };
        let init = if dims.len() == 2 {
            let rows: Vec<String> = (0..dims[0])
                .map(|_| {
                    let row: Vec<String> = (0..dims[1]).map(|_| element(self)).collect();
                    format!("{{{}}}", row.join(", "))
                })
                .collect();
            rows.join(", ")
        } else if self.chance(20) {
            "0".to_owned()
        } else {
            let given = if self.chance(25) {
                1 + self.below(dims[0])
            } else {
                dims[0]
            };
            let items: Vec<String> = (0..given).map(|_| element(self)).collect();
            items.join(", ")
        };
        let sizes: String = dims.iter().map(|d| format!("[{d}]")).collect();
        self.line(format!("int {name}{sizes} = {{{init}}};"));
        let decl = self.decl();
        self.arrays.push(Array {
            name,
            dims,
            home,
            decl,
        });
        true
    }

    fn one_dim(&mut self) -> Option<usize> {
        let found: Vec<usize> = (0..self.arrays.len())
            .filter(|&a| self.arrays[a].dims.len() == 1)
            .collect();
        (!found.is_empty()).then(|| self.pick(&found))
    }

    /// Opens loops over every element of array `a`, returning the element.
    fn open_elements(&mut self, a: usize) -> Option<(Place, i128)> {
        let dims = self.arrays[a].dims.clone();
        let count: usize = dims.iter().product();
        let mult = self.enter_loop(Loop::For, count as i128)?;
        let mut text = self.arrays[a].name.clone();
        for len in dims {
            let i = self.fresh(&["i", "j", "k"]);
            self.open(format!("for (int {i} = 0; {i} < {len}; {i}++)"));
            self.vars.push(Var::counter(i.clone(), 0, len as i128 - 1));
            text = format!("{text}[{i}]");
        }
        let home = self.arrays[a].home;
        Some((Place::int(text, home), mult))
    }

    fn close_elements(&mut self, a: usize, mult: i128) {
        for _ in 0..self.arrays[a].dims.len() {
            let i = self.vars.pop().unwrap();
            self.names.remove(&i.name);
            self.close();
        }
        self.leave_loop(mult);
    }

    fn array_fill(&mut self) -> bool {
        if self.arrays.is_empty() {
            return false;
        }
        let a = self.below(self.arrays.len());
        let Some((element, mult)) = self.open_elements(a) else {
            return false;
        };
        let e = self.expr(2);
        let value = self.fit_place(e, &element);
        self.line(format!("{} = {value};", element.text));
        self.close_elements(a, mult);
        true
    }

    /// Folds every element into a scalar.
    fn array_reduce(&mut self) -> bool {
        if self.arrays.is_empty() || self.scalars(true).is_empty() {
            return false;
        }
        let a = self.below(self.arrays.len());
        let target = self.pick(&self.scalars(true));
        let Some((element, mult)) = self.open_elements(a) else {
            return false;
        };
        let scale = self.pick(&[1, 3, 31]);
        let acc = E::new(target.name.clone(), target.ty, (target.lo, target.hi));
        let scaled = self.arithmetic("*", acc, E::lit(scale));
        let sum = self.arithmetic("+", scaled, element.e());
        let value = self.fit(sum, target.ty, target.lo, target.hi);
        self.line(format!("{} = {value};", target.name));
        self.close_elements(a, mult);
        true
    }

    fn array_algorithm(&mut self) -> bool {
        let Some(a) = self.one_dim() else {
            return false;
        };
        let (name, n, home) = (
            self.arrays[a].name.clone(),
            self.arrays[a].dims[0],
            self.arrays[a].home,
        );
        let Some(mult) = self.enter_loop(Loop::For, n as i128) else {
            return false;
        };
        let i = self.fresh(&["i", "j", "k"]);
        let t = self.fresh(&["tmp", "held", "saved"]);
        match self.below(4) {
            0 => {
                self.open(format!("for (int {i} = 0; {i} < {n} / 2; {i}++)"));
                self.line(format!("int {t} = {name}[{i}];"));
                self.line(format!("{name}[{i}] = {name}[{n} - 1 - {i}];"));
                self.line(format!("{name}[{n} - 1 - {i}] = {t};"));
                self.close();
            }
            1 => {
                self.open(format!("for (int {i} = 1; {i} < {n}; {i}++)"));
                self.line(format!(
                    "{name}[{i}] = ({name}[{i}] + {name}[{i} - 1]) % {};",
                    home + 1
                ));
                self.close();
            }
            2 => {
                self.open(format!("for (int {i} = 0; {i} + 1 < {n}; {i}++)"));
                self.open(format!("if ({name}[{i}] > {name}[{i} + 1])"));
                self.line(format!("int {t} = {name}[{i}];"));
                self.line(format!("{name}[{i}] = {name}[{i} + 1];"));
                self.line(format!("{name}[{i} + 1] = {t};"));
                self.close();
                self.close();
            }
            _ => {
                if self.no_decl {
                    self.leave_loop(mult);
                    return false;
                }
                self.line(format!("int {t} = {name}[0];"));
                self.line(format!("for (int {i} = 0; {i} < {n} - 1; {i}++)"));
                self.line(format!("    {name}[{i}] = {name}[{i} + 1];"));
                self.line(format!("{name}[{n} - 1] = {t};"));
            }
        }
        self.leave_loop(mult);
        true
    }

    fn array_call(&mut self) -> bool {
        let Some(a) = self.one_dim() else {
            return false;
        };
        let (name, n, home) = (
            self.arrays[a].name.clone(),
            self.arrays[a].dims[0] as i128,
            self.arrays[a].home,
        );
        let (call, range) = match self.below(3) {
            0 => (
                format!("{}({name}, {n})", self.helper("sum")),
                (-home * n, home * n),
            ),
            1 => (
                format!("{}({name}, {n})", self.helper("max")),
                (-home, home),
            ),
            _ => {
                let e = self.expr(1);
                let limit = self.fit(e, Ty::Int, -99999, 99999);
                (
                    format!("{}({name}, {n}, {limit})", self.helper("above")),
                    (0, n),
                )
            }
        };
        self.mark(Family::Functions);
        self.store(E::new(call, Ty::Int, range));
        true
    }

    /// Stores a call's result in a new or existing variable.
    fn store(&mut self, e: E) {
        if !self.no_decl && self.chance(50) {
            let name = self.word();
            self.line(format!("int {name} = {};", e.text));
            self.new_var(name, Ty::Int, (e.lo, e.hi), false);
        } else if let Some(place) = self.place(true) {
            let value = self.fit_place(e, &place);
            self.line(format!("{} = {value};", place.text));
        } else {
            self.line(format!("printf(\"%d\\n\", {});", e.text));
        }
    }

    fn array_print(&mut self) -> bool {
        let Some(a) = self.one_dim() else {
            return false;
        };
        if self.mult > 4 {
            return false;
        }
        let (name, n) = (self.arrays[a].name.clone(), self.arrays[a].dims[0]);
        let i = self.fresh(&["i", "j", "k"]);
        self.line(format!("for (int {i} = 0; {i} < {n}; {i}++)"));
        self.line(format!("    printf(\"%d \", {name}[{i}]);"));
        self.line("printf(\"\\n\");");
        self.names.remove(&i);
        true
    }

    // Helpers, written once each when first called.

    fn helper(&mut self, key: &str) -> String {
        if let Some(name) = self.helper_names.get(key) {
            return name.clone();
        }
        let words: &[&str] = match key {
            "sum" => &["sum_of", "add_up", "total_of"],
            "max" => &["max_of", "largest", "top_value"],
            "above" => &["count_above", "how_many_over"],
            "swap" => &["swap", "exchange"],
            "divmod" => &["split_digits", "divide"],
            "clamp" => &["clamp", "limit_to"],
            "fold" => &["fold", "churn", "walk_down"],
            "op_a" => &["combine", "merge_two"],
            "op_b" => &["blend", "mix_two"],
            "tick" => &["tick", "next_id"],
            "note" => &["note", "record"],
            "push" => &["push", "prepend", "cons"],
            "append" => &["append", "push_back"],
            "len" => &["length", "list_len", "count_nodes"],
            "lsum" => &["list_sum", "sum_nodes"],
            "print" => &["print_list", "show"],
            "reverse" => &["reverse", "flip"],
            "pop" => &["pop", "take_first"],
            "remove" => &["remove_value", "drop_value"],
            "free" => &["free_list", "release"],
            "concat" => &["concat", "join_lists"],
            "build" => &["build", "make_list"],
            "node_new" => &["new_node", "make_node", "node_with"],
            "peek" => &["second_value", "peek_next"],
            "bump" => &["bump_next", "touch_next"],
            "consume" => &["consume", "take_value"],
            "maybe_drop" => &["drop_if_above", "maybe_drop"],
            "until" => &["count_until", "run_length"],
            _ => &["total", "make", "shift"],
        };
        let name = match key.split_once(':') {
            Some((kind, _)) => {
                let tag = self.records[self.insts_rec(key)].tag.clone();
                let name = format!("{tag}_{kind}");
                self.names.insert(name.clone());
                name
            }
            None => self.fresh(words),
        };
        self.helper_names.insert(key.to_owned(), name.clone());
        let text = self.helper_text(key, &name);
        self.helpers.push(text);
        name
    }

    fn insts_rec(&self, key: &str) -> usize {
        key.split_once(':').unwrap().1.parse().unwrap()
    }

    fn helper_text(&mut self, key: &str, name: &str) -> String {
        let node = self
            .node
            .as_ref()
            .map(|n| (n.tag.clone(), n.val.clone(), n.next.clone()));
        let (t, v, x) = node.unwrap_or_default();
        match key {
            "sum" => format!(
                "static int {name}(const int *a, int n)\n{{\n    int s = 0;\n    for (int i = 0; i < n; i++)\n        s += a[i];\n    return s;\n}}\n"
            ),
            "max" => format!(
                "static int {name}(const int *a, int n)\n{{\n    int best = a[0];\n    for (int i = 1; i < n; i++)\n        if (a[i] > best)\n            best = a[i];\n    return best;\n}}\n"
            ),
            "above" => format!(
                "static int {name}(const int *a, int n, int limit)\n{{\n    int count = 0;\n    for (int i = 0; i < n; i++) {{\n        if (a[i] <= limit)\n            continue;\n        count++;\n    }}\n    return count;\n}}\n"
            ),
            "swap" => format!(
                "static void {name}(int *p, int *q)\n{{\n    int t = *p;\n    *p = *q;\n    *q = t;\n}}\n"
            ),
            "divmod" => format!(
                "static void {name}(int v, int d, int *quot, int *rem)\n{{\n    *quot = v / d;\n    *rem = v % d;\n}}\n"
            ),
            "clamp" => format!(
                "static int {name}(int v, int lim)\n{{\n    if (v > lim)\n        return lim;\n    if (v < -lim)\n        return -lim;\n    return v;\n}}\n"
            ),
            "fold" => format!(
                "static int {name}(int n, int acc)\n{{\n    if (n <= 0)\n        return acc % 1009;\n    return {name}(n - 1, (acc * 3 + n) % 1009);\n}}\n"
            ),
            "op_a" => {
                format!("static int {name}(int p, int q)\n{{\n    return (p + q) % 1000;\n}}\n")
            }
            "op_b" => format!(
                "static int {name}(int p, int q)\n{{\n    return p > q ? p - q : q - p;\n}}\n"
            ),
            "tick" => format!(
                "static int {name}(int v)\n{{\n    static int calls = 0;\n    calls++;\n    return (v + calls) % 1000;\n}}\n"
            ),
            "note" => {
                let g = self.globals[0].name.clone();
                format!("static void {name}(int v)\n{{\n    {g} = ({g} * 7 + v) % 10007;\n}}\n")
            }
            "push" => format!(
                "static struct {t} *{name}(struct {t} *head, int v)\n{{\n    struct {t} *n = malloc(sizeof *n);\n    if (n == NULL)\n        return head;\n    n->{v} = v;\n    n->{x} = head;\n    return n;\n}}\n"
            ),
            "append" => format!(
                "static void {name}(struct {t} **pp, int v)\n{{\n    while (*pp != NULL)\n        pp = &(*pp)->{x};\n    *pp = malloc(sizeof **pp);\n    if (*pp != NULL) {{\n        (*pp)->{v} = v;\n        (*pp)->{x} = NULL;\n    }}\n}}\n"
            ),
            "len" => format!(
                "static int {name}(const struct {t} *h)\n{{\n    int n = 0;\n    for (; h != NULL; h = h->{x})\n        n++;\n    return n;\n}}\n"
            ),
            "lsum" => format!(
                "static int {name}(const struct {t} *h)\n{{\n    int s = 0;\n    while (h != NULL) {{\n        s += h->{v};\n        h = h->{x};\n    }}\n    return s;\n}}\n"
            ),
            "print" => format!(
                "static void {name}(const struct {t} *h)\n{{\n    for (; h != NULL; h = h->{x})\n        printf(\"%d \", h->{v});\n    printf(\"\\n\");\n}}\n"
            ),
            "reverse" => format!(
                "static struct {t} *{name}(struct {t} *h)\n{{\n    struct {t} *done = NULL;\n    while (h != NULL) {{\n        struct {t} *rest = h->{x};\n        h->{x} = done;\n        done = h;\n        h = rest;\n    }}\n    return done;\n}}\n"
            ),
            "pop" => format!(
                "static int {name}(struct {t} **pp)\n{{\n    struct {t} *first = *pp;\n    if (first == NULL)\n        return 0;\n    int v = first->{v};\n    *pp = first->{x};\n    free(first);\n    return v;\n}}\n"
            ),
            "remove" => format!(
                "static struct {t} *{name}(struct {t} *h, int v)\n{{\n    struct {t} **pp = &h;\n    while (*pp != NULL && (*pp)->{v} != v)\n        pp = &(*pp)->{x};\n    if (*pp != NULL) {{\n        struct {t} *dead = *pp;\n        *pp = dead->{x};\n        free(dead);\n    }}\n    return h;\n}}\n"
            ),
            "free" => format!(
                "static void {name}(struct {t} *h)\n{{\n    while (h != NULL) {{\n        struct {t} *next = h->{x};\n        free(h);\n        h = next;\n    }}\n}}\n"
            ),
            "concat" => format!(
                "static struct {t} *{name}(struct {t} *a, struct {t} *b)\n{{\n    if (a == NULL)\n        return b;\n    struct {t} *last = a;\n    while (last->{x} != NULL)\n        last = last->{x};\n    last->{x} = b;\n    return a;\n}}\n"
            ),
            "build" => {
                let push = self.helper("push");
                format!(
                    "static struct {t} *{name}(int n, int v)\n{{\n    struct {t} *h = NULL;\n    for (int i = 0; i < n; i++)\n        h = {push}(h, (v + i * 7) % 1000);\n    return h;\n}}\n"
                )
            }
            "node_new" => format!(
                "static struct {t} *{name}(int v, struct {t} *next)\n{{\n    struct {t} *n = malloc(sizeof *n);\n    if (n == NULL)\n        exit(3);\n    n->{v} = v;\n    n->{x} = next;\n    return n;\n}}\n"
            ),
            "peek" => format!(
                "static int {name}(const struct {t} *n)\n{{\n    return n->{x} != NULL ? n->{x}->{v} : -1;\n}}\n"
            ),
            "bump" => format!(
                "static int {name}(struct {t} *n, int k)\n{{\n    struct {t} *t = n->{x};\n    t->{v} = (t->{v} + k) % 1000;\n    return n->{x}->{v} + t->{v};\n}}\n"
            ),
            "consume" => format!(
                "static int {name}(struct {t} *n)\n{{\n    int v = n->{v};\n    free(n);\n    return v;\n}}\n"
            ),
            "maybe_drop" => format!(
                "static int {name}(struct {t} *n, int limit)\n{{\n    if (n->{v} > limit) {{\n        free(n);\n        return 1;\n    }}\n    return 0;\n}}\n"
            ),
            "until" => format!(
                "static int {name}(const int *p, const int *end, int limit)\n{{\n    int c = 0;\n    while (p != end && *p <= limit) {{\n        c++;\n        p++;\n    }}\n    return c;\n}}\n"
            ),
            _ => self.record_helper(key, name),
        }
    }

    // Family 4: pointers.

    fn int_targets(&self) -> Vec<usize> {
        (0..self.vars.len())
            .filter(|&v| self.vars[v].writable && self.vars[v].top && self.vars[v].ty == Ty::Int)
            .collect()
    }

    fn pointer_decl(&mut self) -> bool {
        if !self.top_level() {
            return false;
        }
        let name = self.fresh(&["p", "q", "cur", "ptr", "at", "where"]);
        let arrays: Vec<usize> = (0..self.arrays.len())
            .filter(|&a| self.arrays[a].dims.len() == 1)
            .collect();
        if !arrays.is_empty() && self.chance(50) {
            let a = self.pick(&arrays);
            let (array, len) = (self.arrays[a].name.clone(), self.arrays[a].dims[0]);
            let k = self.below(len);
            let init = if self.chance(50) {
                format!("{array} + {k}")
            } else {
                format!("&{array}[{k}]")
            };
            self.line(format!("int *{name} = {init};"));
            self.state.targets.push(Target::Array(a, k, k));
        } else {
            let mut targets = self.int_targets();
            if targets.is_empty() {
                let v = self.word();
                let e = self.expr(1);
                let value = self.fit(e, Ty::Int, -9999, 9999);
                self.line(format!("int {v} = {value};"));
                self.new_var(v, Ty::Int, (-9999, 9999), true);
                targets.push(self.vars.len() - 1);
            }
            let first = self.pick(&targets);
            let second = self.pick(&targets);
            if first != second && self.chance(50) {
                let c = self.cond(0);
                let (a, b) = (
                    self.vars[first].name.clone(),
                    self.vars[second].name.clone(),
                );
                self.line(format!("int *{name} = {c} ? &{a} : &{b};"));
            } else {
                self.line(format!("int *{name} = &{};", self.vars[first].name));
            }
            self.state.targets.push(Target::Vars(
                [first, second]
                    .into_iter()
                    .take(if first == second { 1 } else { 2 })
                    .collect(),
            ));
        }
        let decl = self.decl();
        self.pointers.push((name, decl));
        true
    }

    fn retarget(&mut self) -> bool {
        if self.pointers.is_empty() || self.in_loop() || self.simple > 0 {
            return false;
        }
        let p = self.below(self.pointers.len());
        let name = self.pointers[p].0.clone();
        match self.state.targets[p].clone() {
            Target::Vars(_) => {
                let targets = self.int_targets();
                let t = self.pick(&targets);
                let target = self.vars[t].name.clone();
                if self.top_level() && self.chance(40) {
                    let pp = self.fresh(&["pp", "slot", "handle"]);
                    self.line(format!("int **{pp} = &{name};"));
                    self.line(format!("*{pp} = &{target};"));
                    self.names.insert(pp);
                } else {
                    self.line(format!("{name} = &{target};"));
                }
                self.state.targets[p] = Target::Vars([t].into());
            }
            Target::Array(a, lo, hi) => {
                let (array, len) = (self.arrays[a].name.clone(), self.arrays[a].dims[0]);
                if hi + 1 < len && self.chance(50) {
                    self.line(format!("{name}++;"));
                    self.state.targets[p] = Target::Array(a, lo + 1, hi + 1);
                } else {
                    let k = self.below(len);
                    self.line(format!("{name} = {array} + {k};"));
                    self.state.targets[p] = Target::Array(a, k, k);
                }
            }
        }
        true
    }

    fn assign_through(&mut self) -> bool {
        if self.pointers.is_empty() {
            return false;
        }
        let p = self.below(self.pointers.len());
        let place = self.pointer_place(p, true);
        self.write(place);
        true
    }

    fn write(&mut self, place: Place) {
        let e = self.expr(2);
        let value = self.fit_place(e, &place);
        self.line(format!("{} = {value};", place.text));
    }

    fn pointer_walk(&mut self) -> bool {
        let Some(a) = self.one_dim() else {
            return false;
        };
        let (array, n, home) = (
            self.arrays[a].name.clone(),
            self.arrays[a].dims[0],
            self.arrays[a].home,
        );
        let Some(mult) = self.enter_loop(Loop::For, n as i128) else {
            return false;
        };
        let w = self.fresh(&["w", "it", "walk", "scan"]);
        let element = Place::int(format!("*{w}"), home);
        let end = if self.chance(50) {
            format!("{w} < {array} + {n}")
        } else {
            format!("{w} != {array} + {n}")
        };
        self.open(format!("for (int *{w} = {array}; {end}; {w}++)"));
        self.temps.push(element.clone());
        if self.chance(50) || self.scalars(true).is_empty() {
            let e = self.expr(2);
            let value = self.fit_place(e, &element);
            self.line(format!("{} = {value};", element.text));
        } else {
            let target = self.pick(&self.scalars(true));
            let acc = E::new(target.name.clone(), target.ty, (target.lo, target.hi));
            let sum = self.arithmetic("+", acc, element.e());
            let value = self.fit(sum, target.ty, target.lo, target.hi);
            self.line(format!("{} = {value};", target.name));
        }
        self.temps.pop();
        self.close();
        self.leave_loop(mult);
        self.mark(Family::Arrays);
        true
    }

    /// Two places of equal range that can have their address taken.
    fn addressable(&mut self) -> Vec<(String, i128)> {
        let mut found: Vec<(String, i128)> = self
            .vars
            .iter()
            .filter(|v| v.writable && v.ty == Ty::Int && v.lo == -v.hi)
            .map(|v| (v.name.clone(), v.hi))
            .collect();
        for a in 0..self.arrays.len() {
            if self.arrays[a].dims.len() == 1 {
                let n = self.arrays[a].dims[0];
                let index = self.index(n);
                found.push((
                    format!("{}[{index}]", self.arrays[a].name),
                    self.arrays[a].home,
                ));
            }
        }
        found
    }

    fn swap(&mut self) -> bool {
        let places = self.addressable();
        let a = self.below(places.len());
        let Some((first, home)) = places.get(a).cloned() else {
            return false;
        };
        let same: Vec<String> = places
            .iter()
            .filter(|p| p.1 == home)
            .map(|p| p.0.clone())
            .collect();
        let second = self.pick(&same);
        let swap = self.helper("swap");
        self.line(format!("{swap}(&{first}, &{second});"));
        self.mark(Family::Functions);
        true
    }

    fn out_params(&mut self) -> bool {
        let d = self.range(2, 9);
        let places: Vec<(String, i128)> = self
            .addressable()
            .into_iter()
            .filter(|p| p.1 >= BOUND)
            .collect();
        if places.is_empty() {
            return false;
        }
        let quot = self.pick(&places).0;
        let rem = self.pick(&places).0;
        if quot == rem {
            return false;
        }
        let e = self.expr(1);
        let v = self.fit(e, Ty::Int, -BOUND, BOUND);
        let divide = self.helper("divmod");
        self.line(format!("{divide}({v}, {d}, &{quot}, &{rem});"));
        self.mark(Family::Functions);
        true
    }

    fn pointer_diff(&mut self) -> bool {
        if self.no_decl {
            return false;
        }
        let found: Vec<usize> = (0..self.pointers.len())
            .filter(|&p| matches!(self.state.targets[p], Target::Array(..)))
            .collect();
        if found.is_empty() {
            return false;
        }
        let p = self.pick(&found);
        let Target::Array(a, lo, hi) = self.state.targets[p] else {
            unreachable!()
        };
        let name = self.fresh(&["dist", "offset", "gap", "used"]);
        self.line(format!(
            "int {name} = (int)({} - {});",
            self.pointers[p].0, self.arrays[a].name
        ));
        self.new_var(name, Ty::Int, (lo as i128, hi as i128), false);
        true
    }

    // Family 5: structs.

    fn record(&mut self) -> usize {
        if !self.records.is_empty() && (self.records.len() == 2 || self.chance(70)) {
            return self.below(self.records.len());
        }
        let tag = self.fresh(&["point", "pair", "range", "stats", "entry", "box", "slot"]);
        let mut fields = Vec::new();
        if let Some(inner) = (!self.records.is_empty()).then_some(0) {
            fields.push(Field::Nested(self.fresh(&["inner", "part", "head"]), inner));
        }
        let mut field_names: Vec<&str> = vec![
            "lo", "hi", "x", "y", "count", "total", "id", "w", "flags", "tag",
        ];
        for _ in 0..2 + self.below(2) {
            let f = field_names.remove(self.below(field_names.len())).to_owned();
            let (ty, home) = match self.below(6) {
                0 => (Ty::Short, 9999),
                1 => (Ty::UChar, 0),
                2 => (Ty::UInt, 0),
                _ => (Ty::Int, self.pick(&[999, 99999])),
            };
            fields.push(Field::Scalar(f, ty, home));
        }
        if self.chance(35) {
            let f = field_names.remove(self.below(field_names.len()));
            fields.push(Field::Array(
                format!("{f}s"),
                self.range(2, 4) as usize,
                999,
            ));
        }
        let mut def = format!("struct {tag} {{\n");
        for field in &fields {
            def += &match field {
                Field::Scalar(f, ty, _) => format!("    {} {f};\n", ty.name()),
                Field::Array(f, n, _) => format!("    int {f}[{n}];\n"),
                Field::Nested(f, r) => format!("    struct {} {f};\n", self.records[*r].tag),
            };
        }
        def += "};\n";
        self.top.push(def);
        self.records.push(Record { tag, fields });
        self.records.len() - 1
    }

    fn paths(&self, rec: usize) -> Vec<FieldPath> {
        let mut found = Vec::new();
        for field in &self.records[rec].fields {
            match field {
                Field::Scalar(f, ty, home) => found.push(FieldPath {
                    path: format!(".{f}"),
                    ty: *ty,
                    home: *home,
                    len: None,
                }),
                Field::Array(f, n, home) => found.push(FieldPath {
                    path: format!(".{f}"),
                    ty: Ty::Int,
                    home: *home,
                    len: Some(*n),
                }),
                Field::Nested(f, r) => {
                    for inner in self.paths(*r) {
                        found.push(FieldPath {
                            path: format!(".{f}{}", inner.path),
                            ..inner
                        });
                    }
                }
            }
        }
        found
    }

    fn initializer(&mut self, rec: usize) -> String {
        let fields: Vec<(String, Option<usize>, Option<usize>)> = self.records[rec]
            .fields
            .iter()
            .map(|f| match f {
                Field::Scalar(f, ..) => (f.clone(), None, None),
                Field::Array(f, n, _) => (f.clone(), Some(*n), None),
                Field::Nested(f, r) => (f.clone(), None, Some(*r)),
            })
            .collect();
        let designated = self.chance(30);
        let parts: Vec<String> = fields
            .into_iter()
            .map(|(f, len, nested)| {
                let value = match (len, nested) {
                    (Some(n), _) => {
                        let items: Vec<String> =
                            (0..n).map(|_| self.range(-9, 30).to_string()).collect();
                        format!("{{{}}}", items.join(", "))
                    }
                    (_, Some(r)) => self.initializer(r),
                    _ if self.chance(30) => {
                        let inputs: Vec<String> = self.vars[..self.inputs]
                            .iter()
                            .map(|v| v.name.clone())
                            .collect();
                        self.pick(&inputs)
                    }
                    _ => self.range(0, 40).to_string(),
                };
                if designated {
                    format!(".{f} = {value}")
                } else {
                    value
                }
            })
            .collect();
        format!("{{{}}}", parts.join(", "))
    }

    fn struct_decl(&mut self) -> bool {
        if !self.top_level() {
            return false;
        }
        let rec = self.record();
        let tag = self.records[rec].tag.clone();
        let same: Vec<usize> = (0..self.insts.len())
            .filter(|&i| self.insts[i].rec == rec && self.insts[i].kind != InstKind::Pointer)
            .collect();
        let name = self.fresh(&["s", "cur", "spot", "rec", "best", "info", "item", "first"]);
        let kind = match self.below(4) {
            0 if !same.is_empty() => {
                let i = self.pick(&same);
                let (target, kind) = (self.insts[i].name.clone(), self.insts[i].kind);
                let address = match kind {
                    InstKind::Array(n) => format!("&{target}[{}]", self.below(n)),
                    _ => format!("&{target}"),
                };
                self.line(format!("struct {tag} *{name} = {address};"));
                InstKind::Pointer
            }
            1 if same.iter().any(|&i| self.insts[i].kind == InstKind::Value) => {
                let values: Vec<usize> = same
                    .into_iter()
                    .filter(|&i| self.insts[i].kind == InstKind::Value)
                    .collect();
                let i = self.pick(&values);
                self.line(format!("struct {tag} {name} = {};", self.insts[i].name));
                InstKind::Value
            }
            2 => {
                let n = self.range(2, 3) as usize;
                let items: Vec<String> = (0..n).map(|_| self.initializer(rec)).collect();
                self.line(format!(
                    "struct {tag} {name}[{n}] = {{{}}};",
                    items.join(", ")
                ));
                InstKind::Array(n)
            }
            _ => {
                let init = self.initializer(rec);
                self.line(format!("struct {tag} {name} = {init};"));
                InstKind::Value
            }
        };
        let decl = self.decl();
        self.insts.push(Inst {
            name,
            rec,
            kind,
            decl,
        });
        true
    }

    /// A struct object as an expression of its type, or its address.
    fn struct_object(&mut self, i: usize, address: bool) -> String {
        let (name, kind) = (self.insts[i].name.clone(), self.insts[i].kind);
        match (kind, address) {
            (InstKind::Value, false) => name,
            (InstKind::Value, true) => format!("&{name}"),
            (InstKind::Array(n), false) => format!("{name}[{}]", self.index(n)),
            (InstKind::Array(n), true) => format!("&{name}[{}]", self.index(n)),
            (InstKind::Pointer, false) => format!("*{name}"),
            (InstKind::Pointer, true) => name,
        }
    }

    fn struct_copy(&mut self) -> bool {
        if self.insts.is_empty() {
            return false;
        }
        let i = self.below(self.insts.len());
        let rec = self.insts[i].rec;
        let same: Vec<usize> = (0..self.insts.len())
            .filter(|&j| self.insts[j].rec == rec)
            .collect();
        let j = self.pick(&same);
        let from = self.struct_object(i, false);
        let to = self.struct_object(j, false);
        if from == to {
            return false;
        }
        self.line(format!("{to} = {from};"));
        true
    }

    fn assign_field(&mut self) -> bool {
        if self.insts.is_empty() {
            return false;
        }
        let i = self.below(self.insts.len());
        let place = self.field_place(i);
        self.write(place);
        true
    }

    fn struct_call(&mut self) -> bool {
        if self.insts.is_empty() {
            return false;
        }
        let i = self.below(self.insts.len());
        let rec = self.insts[i].rec;
        match self.below(3) {
            0 => {
                let total = self.helper(&format!("total:{rec}"));
                let range = self.total_range(rec);
                let object = self.struct_object(i, false);
                self.store(E::new(format!("{total}({object})"), Ty::Int, range));
            }
            1 => {
                let make = self.helper(&format!("make:{rec}"));
                let args: Vec<String> = (0..2)
                    .map(|_| {
                        let e = self.expr(1);
                        self.fit(e, Ty::Int, -BOUND, BOUND)
                    })
                    .collect();
                let object = self.struct_object(i, false);
                self.line(format!("{object} = {make}({});", args.join(", ")));
            }
            _ => {
                let shift = self.helper(&format!("shift:{rec}"));
                let e = self.expr(1);
                let d = self.fit(e, Ty::Int, -BOUND, BOUND);
                let object = self.struct_object(i, true);
                self.line(format!("{shift}({object}, {d});"));
            }
        }
        self.mark(Family::Functions);
        true
    }

    /// The scalar fields of a record summed, unsigned ones reduced first.
    fn total_terms(&self, rec: usize) -> (Vec<String>, (i128, i128)) {
        let (mut terms, mut lo, mut hi) = (Vec::new(), 0, 0);
        for path in self.paths(rec).iter().filter(|p| p.len.is_none()) {
            match path.ty {
                Ty::UInt => {
                    terms.push(format!("(int)(v{} % 1000u)", path.path));
                    hi += 999;
                }
                Ty::UChar => {
                    terms.push(format!("v{}", path.path));
                    hi += 255;
                }
                _ => {
                    terms.push(format!("v{}", path.path));
                    lo -= path.home;
                    hi += path.home;
                }
            }
        }
        (terms, (lo, hi))
    }

    fn total_range(&self, rec: usize) -> (i128, i128) {
        self.total_terms(rec).1
    }

    fn record_helper(&mut self, key: &str, name: &str) -> String {
        let rec = self.insts_rec(key);
        let tag = self.records[rec].tag.clone();
        let paths = self.paths(rec);
        let kind = key.split_once(':').unwrap().0;
        let mut body = String::new();
        let assign = |g: &mut Self, target: &str, path: &FieldPath, e: E| -> String {
            let (lo, hi) = path.range();
            format!("{target} = {};", g.fit(e, path.ty, lo, hi))
        };
        match kind {
            "total" => {
                let (terms, _) = self.total_terms(rec);
                format!(
                    "static int {name}(struct {tag} v)\n{{\n    return {};\n}}\n",
                    terms.join(" + ")
                )
            }
            "make" => {
                body += &format!("    struct {tag} r;\n");
                for (n, path) in paths.iter().enumerate() {
                    let param = if n % 2 == 0 { "a" } else { "b" };
                    if let Some(len) = path.len {
                        let e = E::new(
                            format!("{param} + i"),
                            Ty::Int,
                            (-BOUND, BOUND + len as i128),
                        );
                        let line = assign(self, &format!("r{}[i]", path.path), path, e);
                        body += &format!("    for (int i = 0; i < {len}; i++)\n        {line}\n");
                    } else {
                        let e = E::new(param, Ty::Int, (-BOUND, BOUND));
                        body += &format!(
                            "    {}\n",
                            assign(self, &format!("r{}", path.path), path, e)
                        );
                    }
                }
                format!("static struct {tag} {name}(int a, int b)\n{{\n{body}    return r;\n}}\n")
            }
            _ => {
                for path in &paths {
                    let target = format!("p->{}", &path.path[1..]);
                    let (lo, hi) = path.range();
                    if let Some(len) = path.len {
                        let e = E::new(
                            format!("{target}[i] + d"),
                            Ty::Int,
                            (lo - BOUND, hi + BOUND),
                        );
                        let line = assign(self, &format!("{target}[i]"), path, e);
                        body += &format!("    for (int i = 0; i < {len}; i++)\n        {line}\n");
                    } else if path.ty.unsigned() {
                        body += &format!("    {target} += d;\n");
                    } else {
                        let e = E::new(format!("{target} + d"), Ty::Int, (lo - BOUND, hi + BOUND));
                        body += &format!("    {}\n", assign(self, &target, path, e));
                    }
                }
                format!("static void {name}(struct {tag} *p, int d)\n{{\n{body}}}\n")
            }
        }
    }

    // Family 6: heap memory.

    fn buffer_decl(&mut self) -> bool {
        if !self.top_level() {
            return false;
        }
        let name = self.fresh(&["buf", "heap", "cells", "store", "work"]);
        let len = self.range(3, 8) as usize;
        let home = self.pick(&[999, 9999]);
        let zeroed = self.chance(40);
        if zeroed {
            self.line(format!("int *{name} = calloc({len}, sizeof *{name});"));
        } else if self.chance(50) {
            self.line(format!("int *{name} = malloc({len} * sizeof *{name});"));
        } else {
            self.line(format!("int *{name} = malloc(sizeof(int) * {len});"));
        }
        if self.chance(70) {
            self.line(format!("if ({name} == NULL)"));
            self.line("    return 3;");
        }
        // Registered after the fill, which must not read the buffer.
        if !zeroed {
            self.fill_buffer(&name, 0, len, home);
        }
        let decl = self.decl();
        self.buffers.push((name, home, decl));
        self.state.buffers.push((Owned::Live, len));
        true
    }

    fn fill_buffer(&mut self, name: &str, from: usize, to: usize, home: i128) {
        let i = self.fresh(&["i", "j", "k"]);
        self.line(format!("for (int {i} = {from}; {i} < {to}; {i}++)"));
        self.vars
            .push(Var::counter(i.clone(), from as i128, to as i128 - 1));
        let mult = self.mult;
        self.mult *= (to - from) as i128;
        let e = self.expr(1);
        let value = self.fit(e, Ty::Int, -home, home);
        self.mult = mult;
        self.vars.pop();
        self.names.remove(&i);
        self.line(format!("    {name}[{i}] = {value};"));
    }

    /// A realloc, a free on one path, or a free at the top level.
    fn buffer_change(&mut self) -> bool {
        if self.buffers.is_empty() || self.in_loop() || self.simple > 0 {
            return false;
        }
        let b = self.below(self.buffers.len());
        let name = self.buffers[b].0.clone();
        let owned = self.state.buffers[b].0;
        if owned == Owned::Live && self.top_level() && self.chance(50) {
            self.realloc_buffer(b);
            return true;
        }
        if owned == Owned::Null {
            return false;
        }
        if self.top_level() && self.chance(30) {
            self.line(format!("free({name});"));
            self.line(format!("{name} = NULL;"));
        } else {
            let c = self.cond(1);
            self.open(format!("if ({c})"));
            self.line(format!("free({name});"));
            self.line(format!("{name} = NULL;"));
            self.close();
            self.state.buffers[b].0 = Owned::MaybeNull;
            return true;
        }
        self.state.buffers[b].0 = Owned::Null;
        true
    }

    /// Grows live buffer `b` at the top level and fills the new part.
    fn realloc_buffer(&mut self, b: usize) {
        let (name, home, _) = self.buffers[b].clone();
        let len = self.state.buffers[b].1;
        let grown = len + 1 + self.below(4);
        let tmp = self.fresh(&["grown", "bigger", "resized"]);
        self.line(format!(
            "int *{tmp} = realloc({name}, {grown} * sizeof *{name});"
        ));
        self.open(format!("if ({tmp} == NULL)"));
        self.line(format!("free({name});"));
        self.line("return 3;");
        self.close();
        self.line(format!("{name} = {tmp};"));
        self.fill_buffer(&name, len, grown, home);
        self.state.buffers[b] = (Owned::Live, grown);
    }

    /// Uses a buffer that may have been freed only where it is not null.
    fn buffer_guard(&mut self) -> bool {
        let maybe: Vec<usize> = (0..self.buffers.len())
            .filter(|&b| self.state.buffers[b].0 != Owned::Null)
            .collect();
        if maybe.is_empty() {
            return false;
        }
        let b = self.pick(&maybe);
        let name = self.buffers[b].0.clone();
        let saved = self.state.buffers[b].0;
        self.open(format!("if ({name} != NULL)"));
        self.state.buffers[b].0 = Owned::Live;
        self.simple += 1;
        let len = self.state.buffers[b].1;
        let home = self.buffers[b].1;
        let element = Place {
            text: format!("{name}[{}]", self.index(len)),
            ty: Ty::Int,
            lo: -home,
            hi: home,
        };
        let e = self.expr(2);
        let value = self.fit_place(e, &element);
        self.line(format!("{} = {value};", element.text));
        self.scoped(|g| g.statement());
        self.simple -= 1;
        self.state.buffers[b].0 = saved;
        self.close();
        true
    }

    fn node(&mut self) -> Node {
        if self.node.is_none() {
            let tag = self.fresh(&["node", "item", "cell", "link"]);
            let val = self.pick(&["value", "data", "key", "val"]).to_owned();
            let next = self.pick(&["next", "tail", "rest"]).to_owned();
            self.top.push(format!(
                "struct {tag} {{\n    int {val};\n    struct {tag} *{next};\n}};\n"
            ));
            self.node = Some(Node { tag, val, next });
        }
        let n = self.node.as_ref().unwrap();
        Node {
            tag: n.tag.clone(),
            val: n.val.clone(),
            next: n.next.clone(),
        }
    }

    fn list_decl(&mut self) -> bool {
        if !self.top_level() {
            return false;
        }
        let node = self.node();
        if !self.lists.is_empty() && self.chance(30) {
            // An alias into a list, used beside its owner.
            let l = self.below(self.lists.len());
            let name = self.fresh(&["cur", "probe", "seen", "mid"]);
            self.line(format!(
                "struct {} *{name} = {};",
                node.tag, self.lists[l].0
            ));
            self.aliases.push(name);
            self.state.aliases.push((true, [l].into()));
            return true;
        }
        if self.chance(25) {
            let name = self.fresh(&["slots", "bins", "heads"]);
            let n = self.range(2, 3) as usize;
            let nulls = vec!["NULL"; n].join(", ");
            self.line(format!("struct {} *{name}[{n}] = {{{nulls}}};", node.tag));
            self.push_loop(&format!("{name}[i]"), &n.to_string(), n as i128);
            let decl = self.decl();
            for k in 0..n {
                self.lists.push((format!("{name}[{k}]"), decl));
                self.state.lists.push(1);
            }
            return true;
        }
        let name = self.fresh(&["head", "list", "stack", "queue", "chain"]);
        let (init, len) = if self.chance(50) {
            let (bound, hi) = self.bound();
            let e = self.expr(1);
            let v = self.fit(e, Ty::Int, -BOUND, BOUND);
            self.mark(Family::Functions);
            (
                format!("{}({}, {v})", self.helper("build"), bare(&bound)),
                hi,
            )
        } else {
            ("NULL".to_owned(), 0)
        };
        self.line(format!("struct {} *{name} = {init};", node.tag));
        let len = if len == 0 {
            let (bound, hi) = self.bound();
            self.push_loop(&name, &bound, hi);
            hi
        } else {
            len
        };
        let decl = self.decl();
        self.lists.push((name, decl));
        self.state.lists.push(len);
        true
    }

    /// A loop that pushes a node onto `owner` in each iteration.
    fn push_loop(&mut self, owner: &str, bound: &str, iterations: i128) {
        self.line(format!("for (int i = 0; i < {}; i++)", bare(bound)));
        self.vars
            .push(Var::counter("i".to_owned(), 0, iterations - 1));
        let mult = self.mult;
        self.mult *= iterations.max(1);
        let v = self.list_value();
        self.mult = mult;
        self.vars.pop();
        let push = self.helper("push");
        self.line(format!("    {owner} = {push}({owner}, {v});"));
    }

    /// Whether nodes of list `l` may be freed here: in a loop, an alias read
    /// earlier in the body would see them freed in the next iteration.
    fn may_free(&self, l: usize) -> bool {
        !self.in_loop()
            || !self
                .state
                .aliases
                .iter()
                .any(|(alive, deps)| *alive && deps.contains(&l))
    }

    fn kill_aliases(&mut self, l: usize) {
        for (alive, deps) in &mut self.state.aliases {
            if deps.contains(&l) {
                *alive = false;
            }
        }
    }

    fn list_value(&mut self) -> String {
        let e = self.expr(1);
        self.fit(e, Ty::Int, -BOUND, BOUND)
    }

    fn list_op(&mut self) -> bool {
        if self.lists.is_empty() || self.simple > 0 {
            return false;
        }
        let l = self.below(self.lists.len());
        let owner = self.lists[l].0.clone();
        let cap = if self.in_loop() || self.nest > 0 {
            LIST_CAP
        } else {
            self.state.lists[l]
        };
        match self.below(9) {
            0 | 1 if self.state.lists[l] + self.mult <= LIST_CAP => {
                let v = self.list_value();
                if self.chance(50) {
                    let h_push = self.helper("push");
                    self.line(format!("{owner} = {}({owner}, {v});", h_push));
                } else {
                    let h_append = self.helper("append");
                    self.line(format!("{}(&{owner}, {v});", h_append));
                }
                self.state.lists[l] += self.mult;
            }
            2 => {
                let len = self.helper("len");
                self.store(E::new(format!("{len}({owner})"), Ty::Int, (0, cap)));
            }
            3 => {
                let sum = self.helper("lsum");
                self.store(E::new(
                    format!("{sum}({owner})"),
                    Ty::Int,
                    (-cap * BOUND, cap * BOUND),
                ));
            }
            4 if self.mult <= 4 => {
                let h_print = self.helper("print");
                self.line(format!("{}({owner});", h_print));
            }
            5 => {
                let h_reverse = self.helper("reverse");
                self.line(format!("{owner} = {}({owner});", h_reverse));
            }
            6 if self.may_free(l) => {
                let pop = self.helper("pop");
                self.store(E::new(format!("{pop}(&{owner})"), Ty::Int, (-BOUND, BOUND)));
                self.kill_aliases(l);
            }
            7 if self.may_free(l) => {
                let v = self.list_value();
                let h_remove = self.helper("remove");
                self.line(format!("{owner} = {}({owner}, {v});", h_remove));
                self.kill_aliases(l);
            }
            8 if !self.in_loop() && !self.no_decl && !self.scalars(true).is_empty() => {
                // Pop and free until empty, inline.
                let node = self.node();
                let target = self.pick(&self.scalars(true));
                let next = self.fresh(&["after", "following", "nxt"]);
                self.open(format!("while ({owner} != NULL)"));
                self.line(format!(
                    "struct {} *{next} = {owner}->{};",
                    node.tag, node.next
                ));
                let acc = E::new(target.name.clone(), target.ty, (target.lo, target.hi));
                let value = E::new(format!("{owner}->{}", node.val), Ty::Int, (-BOUND, BOUND));
                let sum = self.arithmetic("+", acc, value);
                let sum = self.fit(sum, target.ty, target.lo, target.hi);
                self.line(format!("{} = {sum};", target.name));
                self.line(format!("free({owner});"));
                self.line(format!("{owner} = {next};"));
                self.close();
                self.names.remove(&next);
                self.state.lists[l] = 0;
                self.kill_aliases(l);
            }
            _ => return false,
        }
        self.mark(Family::Functions);
        true
    }

    /// Reads and writes a node through its owner or an alias, where not null.
    fn node_guard(&mut self) -> bool {
        let mut pointers: Vec<String> = self.lists.iter().map(|l| l.0.clone()).collect();
        for (a, name) in self.aliases.iter().enumerate() {
            if self.state.aliases[a].0 {
                pointers.push(name.clone());
            }
        }
        if pointers.is_empty() || self.mult > 64 {
            return false;
        }
        let p = self.pick(&pointers);
        let node = self.node();
        let alias = self.aliases.contains(&p);
        if alias && self.chance(40) {
            self.line(format!("if ({p} != NULL && {p}->{} != NULL)", node.next));
            self.line(format!("    {p} = {p}->{};", node.next));
            return true;
        }
        self.open(format!("if ({p} != NULL)"));
        self.temps.push(Place {
            text: format!("{p}->{}", node.val),
            ty: Ty::Int,
            lo: -BOUND,
            hi: BOUND,
        });
        self.simple += 1;
        let target = self.temps.last().unwrap().clone();
        let e = self.expr(2);
        let value = self.fit_place(e, &target);
        self.line(format!("{} = {value};", target.text));
        self.scoped(|g| g.statement());
        self.simple -= 1;
        self.temps.pop();
        self.close();
        true
    }

    /// Moves the rest of a list out of its first node, joins two lists, or
    /// hands a list to a function that frees it.
    fn list_split(&mut self) -> bool {
        if self.lists.is_empty() || self.in_loop() || self.simple > 0 {
            return false;
        }
        let node = self.node();
        let l = self.below(self.lists.len());
        let owner = self.lists[l].0.clone();
        let empty: Vec<usize> = (0..self.lists.len())
            .filter(|&o| o != l && self.state.lists[o] == 0)
            .collect();
        match self.below(3) {
            0 if !empty.is_empty() || self.top_level() => {
                let rest = if empty.is_empty() || self.chance(30) && self.top_level() {
                    let name = self.fresh(&["rest", "tail", "others", "second"]);
                    self.line(format!("struct {} *{name} = NULL;", node.tag));
                    let decl = self.decl();
                    self.lists.push((name, decl));
                    self.state.lists.push(0);
                    self.lists.len() - 1
                } else {
                    self.pick(&empty)
                };
                let rest_name = self.lists[rest].0.clone();
                self.open(format!("if ({owner} != NULL)"));
                self.line(format!("{rest_name} = {owner}->{};", node.next));
                self.line(format!("{owner}->{} = NULL;", node.next));
                self.close();
                self.state.lists[rest] = self.state.lists[l];
                for (_, deps) in &mut self.state.aliases {
                    if deps.contains(&l) {
                        deps.insert(rest);
                    }
                }
            }
            1 if self.lists.len() > 1 => {
                let other = self.pick(
                    &(0..self.lists.len())
                        .filter(|&o| o != l)
                        .collect::<Vec<_>>(),
                );
                if self.state.lists[l] + self.state.lists[other] > LIST_CAP {
                    return false;
                }
                let other_name = self.lists[other].0.clone();
                let h_concat = self.helper("concat");
                self.line(format!("{owner} = {}({owner}, {other_name});", h_concat));
                self.line(format!("{other_name} = NULL;"));
                self.state.lists[l] += self.state.lists[other];
                self.state.lists[other] = 0;
                for (_, deps) in &mut self.state.aliases {
                    if deps.contains(&other) {
                        deps.insert(l);
                    }
                }
            }
            _ => {
                let h_free = self.helper("free");
                self.line(format!("{}({owner});", h_free));
                self.line(format!("{owner} = NULL;"));
                self.state.lists[l] = 0;
                self.kill_aliases(l);
            }
        }
        self.mark(Family::Functions);
        true
    }

    // Family 7: functions.

    fn recursion(&mut self) -> bool {
        let depth = self.index(10);
        let e = self.expr(1);
        let acc = self.fit(e, Ty::Int, -99999, 99999);
        let fold = self.helper("fold");
        self.store(E::new(
            format!("{fold}({depth}, {acc})"),
            Ty::Int,
            (-1008, 1008),
        ));
        true
    }

    fn function_pointer(&mut self) -> bool {
        if self.no_decl {
            return false;
        }
        let (a, b) = (self.helper("op_a"), self.helper("op_b"));
        let name = self.fresh(&["op", "pick_op", "combine_fn"]);
        let c = self.cond(0);
        if self.chance(50) {
            self.line(format!("int (*{name})(int, int) = {c} ? {a} : {b};"));
        } else {
            self.line(format!("int (*{name})(int, int) = {a};"));
            self.line(format!("if ({c})"));
            self.line(format!("    {name} = {b};"));
        }
        let args: Vec<String> = (0..2).map(|_| self.list_value()).collect();
        self.store(E::new(
            format!("{name}({})", args.join(", ")),
            Ty::Int,
            (-1998, 1998),
        ));
        true
    }

    fn static_counter(&mut self) -> bool {
        let tick = self.helper("tick");
        let v = self.list_value();
        self.store(E::new(format!("{tick}({v})"), Ty::Int, (-BOUND, BOUND)));
        true
    }

    fn global_note(&mut self) -> bool {
        if self.globals.is_empty() {
            let name = self.fresh(&["checksum", "journal", "running"]);
            let start = self.range(0, 99);
            self.top.push(format!("static int {name} = {start};\n"));
            self.globals.push(Var {
                name,
                ty: Ty::Int,
                lo: -10006,
                hi: 10006,
                writable: true,
                top: true,
                decl: 0,
            });
        }
        let note = self.helper("note");
        let v = self.list_value();
        self.line(format!("{note}({v});"));
        true
    }

    fn clamp_call(&mut self) -> bool {
        let clamp = self.helper("clamp");
        let e = self.expr(2);
        let v = self.fit(e, Ty::Int, -99999, 99999);
        let lim = self.range(10, 500);
        self.store(E::new(format!("{clamp}({v}, {lim})"), Ty::Int, (-lim, lim)));
        true
    }

    // Family 8: strings.

    fn string_decl(&mut self) -> bool {
        if !self.top_level() {
            return false;
        }
        let text = self.pick(&TEXTS);
        let cap = text.len() + 8 + self.below(12);
        let name = self.fresh(&["word", "name", "label", "text", "line"]);
        self.line(format!("char {name}[{cap}] = \"{text}\";"));
        let decl = self.decl();
        self.strings.push((name, cap, decl));
        self.state.strings.push((text.len(), text.len()));
        true
    }

    fn string_op(&mut self) -> bool {
        if self.strings.is_empty() {
            return false;
        }
        let s = self.below(self.strings.len());
        let (name, cap, _) = self.strings[s].clone();
        let (lmin, lmax) = self.state.strings[s];
        let i = self.fresh(&["i", "j", "k"]);
        self.names.remove(&i);
        match self.below(7) {
            0 => {
                let add = &self.pick(&TEXTS)[..1 + self.below(3)];
                let grown = lmax + add.len() * self.mult as usize;
                if grown >= cap {
                    return false;
                }
                self.line(format!("strcat({name}, \"{add}\");"));
                let lmin = if self.in_loop() || self.nest > 0 {
                    lmin
                } else {
                    lmin + add.len()
                };
                self.state.strings[s] = (lmin, grown);
            }
            1 if !self.in_loop() => {
                let (from, len) = if self.strings.len() > 1 && self.chance(50) {
                    let o = self.below(self.strings.len());
                    (self.strings[o].0.clone(), self.state.strings[o])
                } else {
                    let text = self.pick(&TEXTS);
                    (format!("\"{text}\""), (text.len(), text.len()))
                };
                if len.1 >= cap || from == name {
                    return false;
                }
                self.line(format!("strcpy({name}, {from});"));
                self.state.strings[s] = len;
            }
            2 if self.mult * cap as i128 <= ITERATIONS => {
                let k = self.letter_shift();
                self.line(format!("for (int {i} = 0; {name}[{i}] != '\\0'; {i}++)"));
                self.line(format!(
                    "    {name}[{i}] = (char)('a' + ({name}[{i}] - 'a' + {k}) % 26);"
                ));
            }
            3 if !self.no_decl && self.mult * cap as i128 <= ITERATIONS => {
                let count = self.fresh(&["hits", "vowels", "matches"]);
                let c = self.pick(&['a', 'e', 'o', 'r', 't']);
                self.line(format!("int {count} = 0;"));
                self.line(format!("for (int {i} = 0; {name}[{i}]; {i}++)"));
                self.line(format!(
                    "    if ({name}[{i}] == '{c}' || {name}[{i}] > 'r')"
                ));
                self.line(format!("        {count}++;"));
                self.new_var(count, Ty::Int, (0, cap as i128), false);
            }
            4 => {
                let range = if self.in_loop() {
                    (lmin as i128, cap as i128 - 1)
                } else {
                    (lmin as i128, lmax as i128)
                };
                self.store(E::new(format!("(int)strlen({name})"), Ty::Int, range));
            }
            5 if lmin > 0 => {
                let idx = self.index(lmin);
                let k = self.letter_shift();
                self.line(format!("{name}[{idx}] = (char)('a' + {k});"));
            }
            _ if self.mult <= 8 => {
                if lmin > 0 && self.chance(50) {
                    let idx = self.index(lmin);
                    self.line(format!("printf(\"%c %s\\n\", {name}[{idx}], {name});"));
                } else {
                    self.line(format!("printf(\"%s\\n\", {name});"));
                }
            }
            _ => return false,
        }
        true
    }

    /// A number from 0 to 25.
    fn letter_shift(&mut self) -> String {
        let e = self.expr(1);
        let k = binary("%", &e, &E::lit(26)).unwrap();
        if k.lo >= 0 {
            k.text
        } else {
            format!("({} + 26) % 26", k.text)
        }
    }

    // Family 9: goto.

    /// A jump to the cleanup at the end of `main`.
    fn goto_out(&mut self) -> bool {
        if let Some(cleanup) = &self.cleanup
            && (cleanup.first != self.decls || cleanup.gotos >= 2)
        {
            return false;
        }
        let c = self.cond(1);
        // Lists and buffers are freed after the label in any state they can
        // be in, so the state there needs no join.
        let cleanup = match self.cleanup.take() {
            Some(mut cleanup) => {
                cleanup.gotos += 1;
                cleanup
            }
            None => Cleanup {
                label: self.fresh(&["out", "done", "cleanup", "finish"]),
                first: self.decls,
                gotos: 1,
            },
        };
        self.line(format!("if ({c})"));
        self.line(format!("    goto {};", cleanup.label));
        self.cleanup = Some(cleanup);
        true
    }

    /// Leaves two nested loops at once.
    fn nested_exit(&mut self) -> bool {
        let (outer, inner) = (self.range(2, 5), self.range(2, 5));
        let Some(mult) = self.enter_loop(Loop::For, outer * inner) else {
            return false;
        };
        let (i, j) = (
            self.fresh(&["i", "row", "r"]),
            self.fresh(&["j", "col", "s"]),
        );
        let label = self.fresh(&["found", "escape", "stop"]);
        self.open(format!("for (int {i} = 0; {i} < {outer}; {i}++)"));
        self.open(format!("for (int {j} = 0; {j} < {inner}; {j}++)"));
        self.scoped(|g| {
            g.vars.push(Var::counter(i.clone(), 0, outer - 1));
            g.vars.push(Var::counter(j.clone(), 0, inner - 1));
            let c = g.cond(1);
            g.line(format!("if ({c})"));
            g.line(format!("    goto {label};"));
            g.statement();
        });
        self.close();
        self.close();
        self.leave_loop(mult);
        self.names.remove(&i);
        self.names.remove(&j);
        self.label(&label);
        if !self.print() {
            self.line("printf(\"-\\n\");");
        }
        true
    }

    /// A loop made of a label and a backward jump.
    fn goto_loop(&mut self) -> bool {
        if self.no_decl {
            return false;
        }
        let k = self.range(2, 4);
        let Some(mult) = self.enter_loop(Loop::Other, k) else {
            return false;
        };
        // Not a C loop: `break` and `continue` cannot leave it.
        self.loops.pop();
        let round = self.fresh(&["round", "attempt", "cycle"]);
        let label = self.fresh(&["again", "retry", "repeat"]);
        self.line(format!("int {round} = 0;"));
        self.label(&label);
        self.scope(false, |g| {
            g.vars.push(Var::counter(round.clone(), 0, k - 1));
            g.no_decl = true;
            g.simple_statement();
            g.no_decl = false;
            g.statement();
        });
        self.line(format!("if (++{round} < {k})"));
        self.line(format!("    goto {label};"));
        self.mult = mult;
        true
    }

    /// Jumps forward over statements, into a block.
    fn goto_skip(&mut self) -> bool {
        let label = self.fresh(&["skip", "resume", "join"]);
        let c = self.cond(1);
        self.line(format!("if ({c})"));
        self.line(format!("    goto {label};"));
        self.simple += 1;
        let into = self.chance(60);
        if into {
            let c = self.cond(1);
            self.open(format!("if ({c})"));
        }
        self.scoped(|g| {
            g.no_decl = true;
            g.statement();
        });
        self.label(&label);
        self.scoped(|g| {
            g.no_decl = true;
            g.statement();
        });
        if into {
            self.close();
        }
        self.simple -= 1;
        true
    }

    // The end of `main`.

    fn finish(&mut self) {
        let first = self.cleanup.as_ref().map_or(usize::MAX, |c| c.first);
        if let Some(cleanup) = self.cleanup.take() {
            self.final_part(|decl| decl >= first);
            self.label(&cleanup.label);
        }
        self.final_part(|decl| decl < first);
        self.line("return 0;");
    }

    /// Prints and frees the objects declared where `chosen` holds.
    fn final_part(&mut self, chosen: impl Fn(usize) -> bool) {
        let vars: Vec<Var> = self
            .vars
            .iter()
            .chain(&self.globals)
            .filter(|v| chosen(v.decl) && v.decl != usize::MAX)
            .cloned()
            .collect();
        let start = vars.len().saturating_sub(4);
        let shown = &vars[start..];
        if !shown.is_empty() {
            let formats: Vec<String> = shown
                .iter()
                .map(|v| format!("{}={}", v.name, v.ty.format()))
                .collect();
            let names: Vec<&str> = shown.iter().map(|v| v.name.as_str()).collect();
            self.line(format!(
                "printf(\"{}\\n\", {});",
                formats.join(" "),
                names.join(", ")
            ));
        } else {
            self.line("printf(\"end\\n\");");
        }
        let arrays: Vec<(String, Vec<usize>)> = self
            .arrays
            .iter()
            .filter(|a| chosen(a.decl))
            .map(|a| (a.name.clone(), a.dims.clone()))
            .collect();
        if let Some((name, dims)) = arrays.last()
            && dims.len() == 1
        {
            self.line(format!("for (int i = 0; i < {}; i++)", dims[0]));
            self.line(format!("    printf(\"%d \", {name}[i]);"));
            self.line("printf(\"\\n\");");
        }
        let strings: Vec<String> = self
            .strings
            .iter()
            .filter(|s| chosen(s.2))
            .map(|s| s.0.clone())
            .collect();
        for s in strings {
            self.line(format!("printf(\"%s\\n\", {s});"));
        }
        let insts: Vec<(String, usize)> = self
            .insts
            .iter()
            .filter(|s| chosen(s.decl) && s.kind == InstKind::Value)
            .map(|s| (s.name.clone(), s.rec))
            .collect();
        if let Some((name, rec)) = insts.last() {
            let paths: Vec<FieldPath> = self
                .paths(*rec)
                .into_iter()
                .filter(|p| p.len.is_none())
                .collect();
            let formats: Vec<&str> = paths.iter().map(|p| p.ty.format()).collect();
            let fields: Vec<String> = paths.iter().map(|p| format!("{name}{}", p.path)).collect();
            self.line(format!(
                "printf(\"{}\\n\", {});",
                formats.join(" "),
                fields.join(", ")
            ));
        }
        let lists: Vec<String> = self
            .lists
            .iter()
            .filter(|l| chosen(l.1))
            .map(|l| l.0.clone())
            .collect();
        for list in lists {
            if self.chance(50) {
                let print = self.helper("print");
                self.line(format!("{print}({list});"));
            }
            let free = self.helper("free");
            self.line(format!("{free}({list});"));
        }
        let buffers: Vec<String> = self
            .buffers
            .iter()
            .filter(|b| chosen(b.2))
            .map(|b| b.0.clone())
            .collect();
        for buffer in buffers {
            self.line(format!("free({buffer});"));
        }
    }
}

/// Shapes 1 to 7: pointers that share an object where a translation would
/// move ownership, each a short idiom at the top level of `main` that frees
/// what it allocates or leaves a list for the cleanup. Shape 8: conversions
/// and side effects that mechanical translation gets wrong.
impl Gen<'_> {
    fn show(&mut self, values: &[String]) {
        let formats = vec!["%d"; values.len()].join(" ");
        self.line(format!("printf(\"{formats}\\n\", {});", values.join(", ")));
    }

    /// A statement between the parts of an idiom, which touches none of it.
    fn filler(&mut self) {
        if self.chance(50) {
            self.simple += 1;
            self.simple_statement();
            self.simple -= 1;
        }
    }

    /// `count` new nodes linked in a chain.
    fn chain_of(&mut self, count: usize) -> String {
        let new = self.helper("node_new");
        let mut text = "NULL".to_owned();
        for _ in 0..count {
            let v = self.list_value();
            text = format!("{new}({v}, {text})");
        }
        text
    }

    /// Hands a chain of `count` nodes to the cleanup, or frees it here.
    fn keep_or_free(&mut self, name: &str, count: usize) {
        if self.chance(50) {
            let decl = self.decl();
            self.lists.push((name.to_owned(), decl));
            self.state.lists.push(count as i128);
            return;
        }
        let next = self.node().next;
        for depth in (0..count).rev() {
            self.line(format!(
                "free({name}{});",
                format!("->{next}").repeat(depth)
            ));
        }
    }

    /// `(field + e) % 1000`, which stays a node value.
    fn moved(&mut self, field: &str) -> String {
        let e = self.operand();
        format!("({field} + {e}) % 1000")
    }

    /// A node value to use inside a larger expression.
    fn operand(&mut self) -> String {
        let v = self.list_value();
        if v.contains(' ') { format!("({v})") } else { v }
    }

    fn holder(&mut self) -> (String, String, String) {
        if self.holder.is_none() {
            let node = self.node().tag;
            let tag = self.fresh(&["holder", "bucket", "owner_rec"]);
            let id = self.pick(&["id", "rank", "slot"]).to_owned();
            let ptr = self.pick(&["item", "ptr", "ref"]).to_owned();
            self.top.push(format!(
                "struct {tag} {{\n    int {id};\n    struct {node} *{ptr};\n}};\n"
            ));
            self.holder = Some((tag, id, ptr));
        }
        self.holder.clone().unwrap()
    }

    fn vector(&mut self) -> (String, String, String) {
        if self.vector.is_none() {
            let tag = self.fresh(&["vec", "span", "block"]);
            let len = self.pick(&["len", "size", "count"]).to_owned();
            let data = self.pick(&["data", "items", "vals"]).to_owned();
            self.top.push(format!(
                "struct {tag} {{\n    int {len};\n    int *{data};\n}};\n"
            ));
            self.vector = Some((tag, len, data));
        }
        self.vector.clone().unwrap()
    }

    /// 1: a pointer field copied into a local without clearing the field,
    /// then both used.
    fn shape_field_copy(&mut self) -> bool {
        if !self.top_level() {
            return false;
        }
        let Node { tag, val, next } = self.node();
        let s = self.fresh(&["head", "base", "anchor", "front"]);
        let init = self.chain_of(2);
        self.line(format!("struct {tag} *{s} = {init};"));
        let t = self.fresh(&["t", "copy", "second", "saved"]);
        let declare = format!("struct {tag} *{t} = {s}->{next};");
        match self.below(5) {
            0 => {
                self.line(&declare);
                self.show(&[format!("{s}->{next} != NULL"), format!("{t}->{val}")]);
                self.filler();
                let value = self.moved(&format!("{t}->{val}"));
                self.line(format!("{t}->{val} = {value};"));
                self.show(&[format!("{s}->{next}->{val}"), format!("{t}->{val}")]);
            }
            1 => {
                let c = self.cond(1);
                self.open(format!("if ({c})"));
                self.line(&declare);
                let e = self.operand();
                self.line(format!("{t}->{val} = ({t}->{val} * 3 + {e}) % 1000;"));
                self.filler();
                self.close();
                self.show(&[format!("{s}->{next}->{val}")]);
            }
            2 => {
                let k = self.range(2, 4);
                self.open(format!("for (int i = 0; i < {k}; i++)"));
                self.line(&declare);
                self.show(&[format!("{t}->{val}")]);
                let e = self.operand();
                self.line(format!(
                    "{s}->{next}->{val} = ({s}->{next}->{val} + i + {e}) % 1000;"
                ));
                self.close();
            }
            3 => {
                self.line(&declare);
                let value = self.moved(&format!("{t}->{val}"));
                self.line(format!("{t}->{val} = {value};"));
                let peek = self.helper("peek");
                self.store(E::new(format!("{peek}({s})"), Ty::Int, (-BOUND, BOUND)));
            }
            _ => {
                self.line(&declare);
                self.show(&[format!("{t}->{val}")]);
                let bump = self.helper("bump");
                let e = self.list_value();
                self.store(E::new(format!("{bump}({s}, {e})"), Ty::Int, (-1998, 1998)));
            }
        }
        self.keep_or_free(&s, 2);
        self.shapes.insert(1);
        true
    }

    /// 2: the same objects reached through other spellings: tables of node
    /// pointers, tables of records holding a pointer, a record holding a
    /// heap array.
    fn shape_spellings(&mut self) -> bool {
        if !self.top_level() {
            return false;
        }
        let Node { tag, val, next } = self.node();
        let n = self.range(2, 4) as usize;
        let (i, j) = (self.index(n), self.index(n));
        let (first, last) = (0, n - 1);
        match self.below(3) {
            0 => {
                let a = self.fresh(&["nodes", "cells", "table"]);
                let new = self.helper("node_new");
                self.line(format!("struct {tag} *{a}[{n}];"));
                let v = self.operand();
                self.line(format!("for (int i = 0; i < {n}; i++)"));
                self.line(format!("    {a}[i] = {new}(({v} + i * 7) % 1000, NULL);"));
                let value = self.moved(&format!("{a}[{j}]->{val}"));
                self.line(format!("{a}[{i}]->{val} = {value};"));
                self.show(&[format!("{a}[0]->{val}"), format!("{a}[{i} + 0]->{val}")]);
                self.line(format!("{a}[{first}]->{next} = {a}[{last}];"));
                let t = self.fresh(&["t", "picked", "held"]);
                self.line(format!("struct {tag} *{t} = {a}[{j}];"));
                self.filler();
                self.show(&[
                    format!("{t}->{val} + {a}[{j}]->{val}"),
                    format!("{a}[{first}]->{next}->{val}"),
                ]);
                if self.chance(60) {
                    self.line(format!("{t} = {a}[{first}];"));
                    self.line(format!("{a}[{first}] = {a}[{last}];"));
                    self.line(format!("{a}[{last}] = {t};"));
                    self.shapes.insert(7);
                }
                self.line(format!("for (int i = 0; i < {n}; i++)"));
                self.line(format!("    free({a}[i]);"));
            }
            1 => {
                let (h, id, ptr) = self.holder();
                let tab = self.fresh(&["tab", "rows", "entries"]);
                let new = self.helper("node_new");
                self.line(format!("struct {h} {tab}[{n}];"));
                let v = self.operand();
                self.open(format!("for (int i = 0; i < {n}; i++)"));
                self.line(format!("{tab}[i].{id} = i;"));
                self.line(format!(
                    "{tab}[i].{ptr} = {new}(({v} + i * 7) % 1000, NULL);"
                ));
                self.close();
                let k = self.below(n);
                let r = self.fresh(&["r", "row", "entry"]);
                self.line(format!("struct {h} *{r} = &{tab}[{k}];"));
                self.show(&[
                    format!("(*{r}).{ptr}->{val}"),
                    format!("(&{tab}[{k}])->{ptr}->{val}"),
                ]);
                let t = self.fresh(&["t", "picked", "held"]);
                self.line(format!("struct {tag} *{t} = {tab}[{i}].{ptr};"));
                let value = self.moved(&format!("{t}->{val}"));
                self.line(format!("{tab}[{j}].{ptr}->{val} = {value};"));
                self.show(&[format!("{tab}[{i}].{ptr}->{val}"), format!("{t}->{val}")]);
                let c = self.cond(1);
                if self.chance(50) {
                    // Moved to another element on one path only.
                    self.open(format!("if ({c})"));
                    self.line(format!("free({tab}[{last}].{ptr});"));
                    self.line(format!("{tab}[{last}].{ptr} = {tab}[{first}].{ptr};"));
                    self.line(format!("{tab}[{first}].{ptr} = NULL;"));
                    self.close();
                } else {
                    // Taken out, and stored back on one path only.
                    self.line(format!("{t} = {tab}[{first}].{ptr};"));
                    self.line(format!("{tab}[{first}].{ptr} = NULL;"));
                    self.line(format!("if ({c})"));
                    self.line(format!("    {tab}[{first}].{ptr} = {t};"));
                    self.line(format!("if ({tab}[{first}].{ptr} == NULL)"));
                    self.line(format!("    free({t});"));
                }
                self.show(&[
                    format!("{tab}[{first}].{ptr} == NULL"),
                    format!("{tab}[{last}].{ptr}->{val} + {tab}[{last}].{id}"),
                ]);
                self.line(format!("for (int i = 0; i < {n}; i++)"));
                self.line(format!("    free({tab}[i].{ptr});"));
            }
            _ => {
                let (vt, len, data) = self.vector();
                let b = self.fresh(&["vec", "store", "bag"]);
                let n = n + 2;
                self.line(format!("struct {vt} {b};"));
                self.line(format!("{b}.{len} = {n};"));
                self.line(format!("{b}.{data} = calloc({n}, sizeof *{b}.{data});"));
                self.line(format!("if ({b}.{data} == NULL)"));
                self.line("    return 3;");
                let view = self.fresh(&["view", "alias", "items"]);
                self.line(format!("int *{view} = {b}.{data};"));
                let (k, k2) = (self.index(n), self.index(n));
                let e = self.list_value();
                self.line(format!("{view}[{k}] = {e};"));
                self.filler();
                self.show(&[format!("{b}.{data}[{k}]"), format!("(&{b})->{data}[{k2}]")]);
                let bp = self.fresh(&["vp", "bp", "ref"]);
                self.line(format!("struct {vt} *{bp} = &{b};"));
                let value = self.moved(&format!("{view}[{k2}]"));
                self.line(format!("(*{bp}).{data}[{k2}] = {value};"));
                let sum = self.helper("sum");
                self.store(E::new(
                    format!("{sum}({b}.{data}, {b}.{len})"),
                    Ty::Int,
                    (-BOUND * n as i128, BOUND * n as i128),
                ));
                self.line(format!("free({b}.{data});"));
            }
        }
        self.shapes.insert(2);
        true
    }

    /// 3: a pointer handed to a function chosen by a condition, with
    /// cleanup that depends on it; an owner freed by a function only on some
    /// condition, which it tells by its return value.
    fn shape_chosen_owner(&mut self) -> bool {
        if !self.top_level() {
            return false;
        }
        let Node { tag, val, next } = self.node();
        let x = self.fresh(&["left", "one", "x_node"]);
        let init = self.chain_of(1);
        self.line(format!("struct {tag} *{x} = {init};"));
        let got = self.fresh(&["got", "taken", "result"]);
        match self.below(3) {
            0 => {
                let y = self.fresh(&["right", "other", "y_node"]);
                let init = self.chain_of(1);
                self.line(format!("struct {tag} *{y} = {init};"));
                let k = self.fresh(&["k", "which", "flag"]);
                let c = self.cond(1);
                self.line(format!("int {k} = {c};"));
                let consume = self.helper("consume");
                self.line(format!("int {got} = {consume}({k} ? {x} : {y});"));
                self.line(format!("if ({k})"));
                self.line(format!("    free({y});"));
                self.line("else");
                self.line(format!("    free({x});"));
            }
            1 => {
                let s = self.fresh(&["pair", "holder_node", "two"]);
                let init = self.chain_of(2);
                self.line(format!("struct {tag} *{s} = {init};"));
                let k = self.fresh(&["k", "which", "flag"]);
                let c = self.cond(1);
                self.line(format!("int {k} = {c};"));
                let consume = self.helper("consume");
                self.line(format!("int {got} = {consume}({k} ? {s}->{next} : {x});"));
                self.open(format!("if ({k})"));
                self.line(format!("{s}->{next} = NULL;"));
                self.line(format!("free({x});"));
                self.close();
                self.show(&[
                    got.clone(),
                    format!("{s}->{next} != NULL ? {s}->{next}->{val} : -1"),
                ]);
                self.line(format!("free({s}->{next});"));
                self.line(format!("free({s});"));
            }
            _ => {
                let drop = self.helper("maybe_drop");
                let e = self.list_value();
                self.line(format!("int {got} = {drop}({x}, {e});"));
                self.open(format!("if (!{got})"));
                let value = self.moved(&format!("{x}->{val}"));
                self.line(format!("{x}->{val} = {value};"));
                self.show(&[format!("{x}->{val}")]);
                self.line(format!("free({x});"));
                self.close();
            }
        }
        self.new_var(got, Ty::Int, (-BOUND, BOUND), false);
        self.shapes.insert(3);
        true
    }

    /// 4: a list changed through a pointer to a pointer.
    fn shape_cursor(&mut self) -> bool {
        if !self.top_level() {
            return false;
        }
        let Node { tag, val, next } = self.node();
        let pp = self.fresh(&["pp", "link", "cursor"]);
        let new = self.helper("node_new");
        let choice = self.below(3);
        if choice == 0 || self.lists.is_empty() {
            let s = self.fresh(&["head", "base", "anchor", "front"]);
            let init = self.chain_of(2);
            self.line(format!("struct {tag} *{s} = {init};"));
            self.line(format!("struct {tag} **{pp} = &{s}->{next};"));
            self.show(&[format!("(*{pp})->{val}")]);
            let v = self.operand();
            if self.chance(50) {
                self.line(format!(
                    "*{pp} = {new}(({v} + (*{pp})->{val} % 2) % 1000, *{pp});"
                ));
            } else {
                self.line(format!("*{pp} = {new}({v}, *{pp});"));
            }
            self.show(&[
                format!("(*{pp})->{val}"),
                format!("{s}->{next}->{next}->{val}"),
            ]);
            self.keep_or_free(&s, 3);
        } else {
            let l = self.below(self.lists.len());
            let owner = self.lists[l].0.clone();
            self.line(format!("struct {tag} **{pp} = &{owner};"));
            if choice == 1 && self.state.lists[l] < LIST_CAP {
                let v = self.list_value();
                self.line(format!("while (*{pp} != NULL && (*{pp})->{val} < {v})"));
                self.line(format!("    {pp} = &(*{pp})->{next};"));
                let v = self.list_value();
                self.line(format!("*{pp} = {new}({v}, *{pp});"));
                self.state.lists[l] += 1;
            } else {
                let k = self.range(2, 4);
                let dead = self.fresh(&["dead", "gone", "victim"]);
                self.open(format!("while (*{pp} != NULL)"));
                self.open(format!("if ((*{pp})->{val} % {k} == 0)"));
                self.line(format!("struct {tag} *{dead} = *{pp};"));
                self.line(format!("*{pp} = {dead}->{next};"));
                self.line(format!("free({dead});"));
                self.reopen("else");
                self.line(format!("{pp} = &(*{pp})->{next};"));
                self.close();
                self.close();
                self.kill_aliases(l);
            }
            let print = self.helper("print");
            self.line(format!("{print}({owner});"));
        }
        self.shapes.insert(4);
        true
    }

    /// 5: nested paths through two and three links.
    fn shape_nested(&mut self) -> bool {
        if !self.top_level() {
            return false;
        }
        let Node { tag, val, next } = self.node();
        let a = self.fresh(&["head", "base", "anchor", "front"]);
        let init = self.chain_of(3);
        self.line(format!("struct {tag} *{a} = {init};"));
        let b = self.fresh(&["mid", "middle", "b_node"]);
        self.line(format!("struct {tag} *{b} = {a}->{next};"));
        self.show(&[
            format!("{a}->{next}->{val}"),
            format!("{b}->{next}->{val}"),
            format!("{a}->{next}->{next}->{val}"),
        ]);
        let value = self.moved(&format!("{b}->{val}"));
        self.line(format!("{a}->{next}->{next}->{val} = {value};"));
        self.filler();
        self.line(format!(
            "{b}->{val} = ({a}->{next}->{next}->{val} - {b}->{val}) % 1000;"
        ));
        self.show(&[format!("{a}->{next}->{val}")]);
        self.keep_or_free(&a, 3);
        self.shapes.insert(5);
        true
    }

    /// 6: pointers into the middle of an array and one past its end.
    fn shape_inner_pointer(&mut self) -> bool {
        if !self.top_level() {
            return false;
        }
        let live: Vec<usize> = (0..self.buffers.len())
            .filter(|&b| self.state.buffers[b].0 == Owned::Live)
            .collect();
        let use_buffer = !live.is_empty() && self.chance(50);
        if !use_buffer && self.one_dim().is_none() {
            // A one-dimensional array to point into.
            while !self.array_decl() || self.arrays.last().unwrap().dims.len() != 1 {}
        }
        let (name, n, home) = if use_buffer {
            let b = self.pick(&live);
            (
                self.buffers[b].0.clone(),
                self.state.buffers[b].1,
                self.buffers[b].1,
            )
        } else {
            let a = self.one_dim().unwrap();
            let array = &self.arrays[a];
            (array.name.clone(), array.dims[0], array.home)
        };
        let k = self.below(n);
        match self.below(3) {
            0 => {
                let m = 1 + self.below(n - k);
                let sum = self.helper("sum");
                let start = if self.chance(50) {
                    format!("&{name}[{k}]")
                } else {
                    format!("{name} + {k}")
                };
                self.mark(Family::Functions);
                self.store(E::new(
                    format!("{sum}({start}, {m})"),
                    Ty::Int,
                    (-home * m as i128, home * m as i128),
                ));
            }
            1 => {
                let until = self.helper("until");
                let e = self.list_value();
                self.mark(Family::Functions);
                self.store(E::new(
                    format!("{until}(&{name}[{k}], &{name}[{n}], {e})"),
                    Ty::Int,
                    (0, (n - k) as i128),
                ));
            }
            _ => {
                let mid = self.fresh(&["mid", "inner", "part"]);
                self.line(format!("int *{mid} = {name} + {k};"));
                let j = self.index(n);
                let e = self.expr(1);
                let value = self.fit(e, Ty::Int, -home, home);
                self.line(format!("{name}[{j}] = {value};"));
                self.line(format!("*{mid} = (*{mid} + {name}[{j}]) % {};", home + 1));
                let back = if k > 0 {
                    format!("*({mid} - 1)")
                } else {
                    format!("{mid}[0]")
                };
                self.show(&[format!("{name}[{k}]"), back]);
            }
        }
        self.shapes.insert(6);
        true
    }

    /// 7: owners swapped, rotated or moved to another container; a buffer
    /// grown while an index into it is kept.
    fn shape_swap_owners(&mut self) -> bool {
        if !self.top_level() {
            return false;
        }
        let live: Vec<usize> = (0..self.buffers.len())
            .filter(|&b| self.state.buffers[b].0 == Owned::Live)
            .collect();
        if !live.is_empty() && self.chance(30) {
            let b = self.pick(&live);
            let (name, len) = (self.buffers[b].0.clone(), self.state.buffers[b].1);
            let at = self.fresh(&["at", "keep", "spot"]);
            let index = self.index(len);
            self.line(format!("int {at} = {index};"));
            self.realloc_buffer(b);
            self.show(&[format!("{name}[{at}]")]);
            self.new_var(at, Ty::Int, (0, len as i128 - 1), false);
            self.shapes.insert(7);
            return true;
        }
        while self.lists.len() < 2 {
            self.list_decl();
        }
        let tag = self.node().tag;
        let count = self.lists.len().min(2 + self.below(2));
        let mut chosen: Vec<usize> = (0..self.lists.len()).collect();
        while chosen.len() > count {
            chosen.remove(self.below(chosen.len()));
        }
        let names: Vec<String> = chosen.iter().map(|&l| self.lists[l].0.clone()).collect();
        let empty = chosen.iter().position(|&l| self.state.lists[l] == 0);
        if let (Some(e), true) = (empty, self.chance(50)) {
            // Stored into the empty container, removed from the first.
            let from = (e + 1) % chosen.len();
            self.line(format!("{} = {};", names[e], names[from]));
            self.line(format!("{} = NULL;", names[from]));
            self.state.lists[chosen[e]] = self.state.lists[chosen[from]];
            self.state.lists[chosen[from]] = 0;
        } else {
            let t = self.fresh(&["t", "swap_tmp", "spare"]);
            self.line(format!("struct {tag} *{t} = {};", names[0]));
            for w in 1..names.len() {
                self.line(format!("{} = {};", names[w - 1], names[w]));
            }
            self.line(format!("{} = {t};", names[names.len() - 1]));
            let lens: Vec<i128> = chosen.iter().map(|&l| self.state.lists[l]).collect();
            for (w, &l) in chosen.iter().enumerate() {
                self.state.lists[l] = lens[(w + 1) % lens.len()];
            }
        }
        // An alias into any of them now points into a list another one owns.
        for (_, deps) in &mut self.state.aliases {
            if chosen.iter().any(|l| deps.contains(l)) {
                deps.extend(chosen.iter().copied());
            }
        }
        let len = self.helper("len");
        self.show(&[format!("{len}({})", names[0])]);
        self.shapes.insert(7);
        true
    }

    // Shape 8: integers.

    /// A comparison of a signed with an unsigned value.
    fn compare_print(&mut self) -> bool {
        if self.mult > 8 {
            return false;
        }
        let a = self.expr(1);
        let mut b = self.place(false).map_or_else(|| self.expr(1), |p| p.e());
        if a.ty.unsigned() == b.ty.unsigned() {
            let range = b.range_as(Ty::UInt);
            b = E::new(format!("(unsigned){}", b.text), Ty::UInt, range);
        }
        let op = self.pick(&["<", "<=", ">", ">="]);
        self.show(&[format!("{} {op} {}", a.text, b.text)]);
        self.shapes.insert(8);
        true
    }

    /// A product that wraps in 32 bits beside the same product in 64.
    fn widen_pair(&mut self) -> bool {
        if self.no_decl {
            return false;
        }
        let unsigned = |g: &mut Self| {
            let e = g.expr(1);
            if e.ty == Ty::UInt {
                e.text
            } else {
                format!("(unsigned){}", e.text)
            }
        };
        let (a, b) = (unsigned(self), unsigned(self));
        let (narrow, wide) = (self.word(), self.word());
        self.line(format!("unsigned long {narrow} = {a} * {b};"));
        self.line(format!("unsigned long {wide} = (unsigned long){a} * {b};"));
        for name in [narrow, wide] {
            self.new_var(name, Ty::ULong, Ty::ULong.range(), true);
        }
        self.shapes.insert(8);
        true
    }

    /// Writes with a post-increment in the index.
    fn post_increment(&mut self) -> bool {
        if self.no_decl {
            return false;
        }
        let Some(a) = self.one_dim() else {
            return false;
        };
        let (name, n, home) = (
            self.arrays[a].name.clone(),
            self.arrays[a].dims[0],
            self.arrays[a].home,
        );
        let pos = self.fresh(&["pos", "fill", "next_at"]);
        self.line(format!("int {pos} = 0;"));
        let m = 1 + self.below(n.min(4));
        for _ in 0..m {
            let e = self.expr(1);
            let value = self.fit(e, Ty::Int, -home, home);
            self.line(format!("{name}[{pos}++] = {value};"));
        }
        self.new_var(pos, Ty::Int, (m as i128, m as i128), false);
        self.shapes.insert(8);
        true
    }

    /// Conditions whose right operand counts, so short-circuiting shows.
    fn short_circuit(&mut self) -> bool {
        if self.no_decl {
            return false;
        }
        let hits = self.fresh(&["hits", "checks", "tests"]);
        self.line(format!("int {hits} = 0;"));
        let n = 2 + self.below(2);
        for _ in 0..n {
            let c = self.cond(0);
            let side = if self.chance(50) {
                format!("{c} && ++{hits} > 1")
            } else {
                format!("{c} || {hits}++ < 1")
            };
            self.open(format!("if ({side})"));
            self.filler();
            self.show(std::slice::from_ref(&hits));
            self.close();
        }
        self.new_var(hits, Ty::Int, (0, n as i128), false);
        self.shapes.insert(8);
        true
    }

    /// `a = b = value`, where `a` receives `b`'s value converted.
    fn chain(&mut self) -> bool {
        let vars = self.scalars(true);
        if vars.len() < 2 {
            return false;
        }
        let (a, b) = (self.pick(&vars), self.pick(&vars));
        let fits = a.ty.unsigned() || (b.lo >= a.lo && b.hi <= a.hi);
        if a.name == b.name || !fits {
            return false;
        }
        let e = self.expr(2);
        let value = self.fit(e, b.ty, b.lo, b.hi);
        self.line(format!("{} = {} = {value};", a.name, b.name));
        self.shapes.insert(8);
        true
    }

    /// A loop whose unsigned counter counts down to zero.
    fn countdown(&mut self) -> bool {
        let small: Vec<Var> = self
            .vars
            .iter()
            .filter(|v| v.lo >= 0 && v.hi >= 1 && v.hi <= 12)
            .cloned()
            .collect();
        let (bound, hi) = if !small.is_empty() && self.chance(50) {
            let v = self.pick(&small);
            (v.name, v.hi)
        } else {
            let k = self.range(2, 7);
            (k.to_string(), k)
        };
        let Some(mult) = self.enter_loop(Loop::For, hi) else {
            return false;
        };
        let k = self.fresh(&["k", "down", "left_n"]);
        let (header, lo, top) = if self.chance(50) {
            (
                format!("for (unsigned {k} = {bound}; {k}-- > 0;)"),
                0,
                hi - 1,
            )
        } else {
            (
                format!("for (unsigned {k} = {bound}; {k} > 0; {k}--)"),
                1,
                hi,
            )
        };
        self.open(header);
        self.scoped(|g| {
            let mut counter = Var::counter(k.clone(), lo, top.max(lo));
            counter.ty = Ty::UInt;
            g.vars.push(counter);
            g.body();
        });
        self.names.remove(&k);
        self.close();
        self.leave_loop(mult);
        self.shapes.insert(8);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_depend_on_seed_and_index_only() {
        let a = candidate(7, 3);
        assert_eq!(a.source, candidate(7, 3).source);
        assert_eq!(a.inputs, candidate(7, 3).inputs);
        assert_ne!(a.source, candidate(8, 3).source);
        assert_ne!(a.source, candidate(7, 4).source);
    }

    #[test]
    fn candidates_look_like_programs() {
        let mut families: BTreeMap<Family, usize> = BTreeMap::new();
        for seed in [1, 0xfeed] {
            for index in 0..300 {
                let c = candidate(seed, index);
                let lines = c.source.lines().count();
                assert!(
                    (LINES.0..=LINES.1).contains(&lines),
                    "{lines} lines:\n{}",
                    c.source
                );
                for (open, close) in [('{', '}'), ('(', ')'), ('[', ']')] {
                    assert_eq!(
                        c.source.matches(open).count(),
                        c.source.matches(close).count(),
                        "{}",
                        c.source
                    );
                }
                assert_eq!(c.inputs.len(), 4);
                assert!(!c.families.is_empty(), "{:?}", c.families);
                for family in c.families {
                    *families.entry(family).or_default() += 1;
                }
            }
        }
        for (family, _) in FAMILIES {
            assert!(
                families.get(&family).copied().unwrap_or(0) >= 60,
                "{families:?}"
            );
        }
    }

    #[test]
    fn most_candidates_are_valid_references() {
        let scratch = eval::Scratch::new("generate-test").unwrap();
        let passed = std::thread::scope(|scope| {
            let workers: Vec<_> = (0..4)
                .map(|w| {
                    let scratch = &scratch;
                    scope.spawn(move || {
                        (w..20)
                            .step_by(4)
                            .filter(|&i| {
                                check(&candidate(0x5eed, i), &scratch.join(i.to_string())).is_ok()
                            })
                            .count()
                    })
                })
                .collect();
            workers
                .into_iter()
                .map(|w| w.join().unwrap())
                .sum::<usize>()
        });
        assert!(passed >= 16, "{passed} of 20 passed");
    }

    #[test]
    fn arguments_have_defaults() {
        let parsed = |args: &[&str]| parse_args(args.iter().map(|arg| arg.to_string()));
        let args = parsed(&["private"]).unwrap();
        assert_eq!((args.count, args.jobs), (COUNT, 2));
        let args = parsed(&["private", "--count", "5", "--jobs", "3"]).unwrap();
        assert_eq!((args.count, args.jobs), (5, 3));
        assert!(parsed(&["private", "--count", "0"]).is_none());
        assert!(parsed(&[]).is_none());
    }
}
