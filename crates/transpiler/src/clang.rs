//! The front end: builds `c::Program` from Clang's JSON syntax tree. It holds
//! no translation rule; a construct it cannot represent is `Unsupported`.

use crate::Unsupported;
use crate::c::{
    BinOp, Enumerator, Expr, ExprKind, Field, Function, Program, Record, RecordId, Stmt, Type,
    UnOp, Var, VarId, VarKind,
};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

type R<T> = Result<T, Unsupported>;

/// Reads the syntax tree of one C file.
pub fn program(ast: &Value) -> R<Program> {
    let mut reader = Reader::default();
    for decl in children(ast) {
        reader.top_level(decl)?;
    }
    Ok(reader.program)
}

#[derive(Default)]
struct Reader {
    program: Program,
    /// A typedef's name to its type, or to why it has none here.
    typedefs: HashMap<String, Result<Type, String>>,
    /// `struct NAME` and `union NAME` to the record.
    tags: HashMap<String, RecordId>,
    /// The records whose members have been read.
    defined: HashSet<RecordId>,
    /// The type of each enumeration, by `enum NAME` and by Clang's id.
    enums: HashMap<String, Type>,
    /// The types of the type strings read so far.
    types: HashMap<String, Result<Type, String>>,
    // Clang's ids of declarations.
    records: HashMap<String, RecordId>,
    fields: HashMap<String, (RecordId, usize)>,
    enumerators: HashMap<String, usize>,
    vars: HashMap<String, VarId>,
    /// File-scope variables by name: one may be declared several times.
    globals: HashMap<String, VarId>,
    /// The labels of the function being read, by Clang's id.
    labels: HashMap<String, String>,
}

/// Whether an attribute of this kind changes where a record's members lie.
fn changes_layout(kind: &str) -> bool {
    matches!(kind, "PackedAttr" | "AlignedAttr" | "MaxFieldAlignmentAttr")
}

fn children(node: &Value) -> &[Value] {
    node["inner"].as_array().map_or(&[], Vec::as_slice)
}

fn kind(node: &Value) -> &str {
    node["kind"].as_str().unwrap_or_default()
}

fn text<'a>(node: &'a Value, key: &str) -> &'a str {
    node[key].as_str().unwrap_or_default()
}

/// The node's offset in the source file, or 0.
fn at(node: &Value) -> u32 {
    let begin = &node["range"]["begin"];
    let offset = begin["offset"]
        .as_u64()
        .or_else(|| begin["expansionLoc"]["offset"].as_u64());
    offset
        .and_then(|offset| u32::try_from(offset).ok())
        .unwrap_or(0)
}

/// Whether the program's own file declares this, rather than a header or
/// the compiler. Clang names the including file on every location in a
/// header.
fn own(decl: &Value) -> bool {
    let loc = &decl["loc"];
    decl["isImplicit"] != true
        && [loc, &loc["spellingLoc"], &loc["expansionLoc"]]
            .iter()
            .all(|loc| loc["includedFrom"].is_null())
}

fn unsupported<T>(what: impl Into<String>, node: &Value) -> R<T> {
    Err(Unsupported::new(what, at(node)))
}

impl Reader {
    fn top_level(&mut self, decl: &Value) -> R<()> {
        match kind(decl) {
            "TypedefDecl" => self.typedef(decl)?,
            "RecordDecl" => self.record(decl, own(decl))?,
            "EnumDecl" => self.enumeration(decl)?,
            "VarDecl" if own(decl) => {
                self.var(decl, VarKind::Global)?;
            }
            "FunctionDecl" if own(decl) => self.function(decl)?,
            _ => {}
        }
        Ok(())
    }

    fn typedef(&mut self, decl: &Value) -> R<()> {
        let name = text(decl, "name").to_owned();
        let ty = match children(decl).first() {
            Some(node) => self.type_node(node),
            None => self.type_str(text(&decl["type"], "qualType")),
        };
        // `typedef struct { ... } NAME;` names the record.
        if let Ok(Type::Record(id)) = ty
            && self.program.records[id].name.is_empty()
        {
            self.program.records[id].name = name.clone();
        }
        // Types are found by name, so a name has one meaning in the whole
        // program.
        if self.typedefs.get(&name).is_some_and(|known| *known != ty) {
            return unsupported(format!("the typedef `{name}` with two meanings"), decl);
        }
        self.typedefs.insert(name, ty);
        Ok(())
    }

    /// The type of a type node, which Clang prints only below a typedef.
    fn type_node(&mut self, node: &Value) -> Result<Type, String> {
        let inner = children(node).first();
        let spelled = text(&node["type"], "qualType");
        let below = |reader: &mut Self| match inner {
            Some(inner) => reader.type_node(inner),
            None => reader.type_str(spelled),
        };
        match kind(node) {
            "PointerType" => Ok(Type::Pointer(Box::new(below(self)?))),
            "ConstantArrayType" => Ok(Type::Array(Box::new(below(self)?), node["size"].as_u64())),
            "ElaboratedType" | "ParenType" | "QualType" => below(self),
            "RecordType" => match self.records.get(text(&node["decl"], "id")) {
                Some(&id) => Ok(Type::Record(id)),
                // The definition may come after the typedef.
                None => self.type_str(spelled),
            },
            "EnumType" => match self.enums.get(text(&node["decl"], "id")) {
                Some(ty) => Ok(ty.clone()),
                None => self.type_str(spelled),
            },
            "TypedefType" => self.type_str(text(&node["decl"], "name")),
            _ => self.type_str(spelled),
        }
    }

    fn record(&mut self, decl: &Value, own: bool) -> R<()> {
        let name = text(decl, "name");
        let union = text(decl, "tagUsed") == "union";
        let tag = format!("{} {name}", text(decl, "tagUsed"));
        let known = (!name.is_empty())
            .then(|| self.tags.get(&tag).copied())
            .flatten();
        let id = known.unwrap_or_else(|| {
            self.program.records.push(Record {
                name: name.to_owned(),
                union,
                fields: Vec::new(),
                own: false,
            });
            self.program.records.len() - 1
        });
        if !name.is_empty() {
            self.tags.insert(tag, id);
        }
        self.records.insert(text(decl, "id").to_owned(), id);
        if !own || decl["completeDefinition"] != true {
            return Ok(());
        }
        // Records are found by tag, so a tag has one meaning in the whole
        // program.
        if !self.defined.insert(id) {
            return unsupported(format!("the tag `{name}` with two meanings"), decl);
        }
        self.program.records[id].own = true;
        for member in children(decl) {
            match kind(member) {
                "FieldDecl" => {
                    if children(member)
                        .iter()
                        .any(|part| changes_layout(kind(part)))
                    {
                        return unsupported("a member with its own alignment", member);
                    }
                    let field = Field {
                        name: text(member, "name").to_owned(),
                        ty: self.ty(member)?,
                        bits: children(member)
                            .first()
                            .filter(|_| member["isBitfield"] == true)
                            .and_then(|width| text(width, "value").parse().ok()),
                    };
                    let index = self.program.records[id].fields.len();
                    self.fields
                        .insert(text(member, "id").to_owned(), (id, index));
                    self.program.records[id].fields.push(field);
                }
                "RecordDecl" => self.record(member, true)?,
                "EnumDecl" => self.enumeration(member)?,
                "IndirectFieldDecl" => {
                    return unsupported("a member of an unnamed struct or union", member);
                }
                attribute if changes_layout(attribute) => {
                    return unsupported("a record that is packed or aligned", decl);
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn enumeration(&mut self, decl: &Value) -> R<()> {
        let mut next: i128 = 0;
        let (mut least, mut greatest) = (0, 0);
        for constant in children(decl) {
            if kind(constant) != "EnumConstantDecl" {
                continue;
            }
            let value = match children(constant).first() {
                // Clang prints the value of some constants only.
                Some(written) => match text(written, "value").parse().ok().or_else(|| {
                    let written = self.expr(written).ok()?;
                    written.constant(&self.program)
                }) {
                    Some(value) => value,
                    None if own(decl) => {
                        return unsupported("an enumerator without a constant value", constant);
                    }
                    // A header's enumeration that the program cannot use.
                    None => return Ok(()),
                },
                None => next,
            };
            next = value + 1;
            (least, greatest) = (least.min(value), greatest.max(value));
            self.enumerators.insert(
                text(constant, "id").to_owned(),
                self.program.enumerators.len(),
            );
            self.program.enumerators.push(Enumerator {
                name: text(constant, "name").to_owned(),
                value,
            });
        }
        // Clang's choice on x86-64. A wider enumeration gets no type, so
        // its use is refused.
        let signed = least < 0;
        let fits = match signed {
            true => least >= i128::from(i32::MIN) && greatest <= i128::from(i32::MAX),
            false => greatest <= i128::from(u32::MAX),
        };
        if fits {
            let ty = Type::Int { bits: 32, signed };
            let name = text(decl, "name");
            if !name.is_empty() {
                // As for records, a tag has one meaning.
                let tag = format!("enum {name}");
                if self.enums.get(&tag).is_some_and(|known| *known != ty) {
                    return unsupported(format!("the tag `{name}` with two meanings"), decl);
                }
                self.enums.insert(tag, ty.clone());
            }
            self.enums.insert(text(decl, "id").to_owned(), ty);
        }
        Ok(())
    }

    fn var(&mut self, decl: &Value, storage: VarKind) -> R<VarId> {
        let name = text(decl, "name");
        let known = match storage {
            VarKind::Global => self.globals.get(name).copied(),
            _ => None,
        };
        let id = match known {
            Some(id) => id,
            None => {
                let ty = self.ty(decl)?;
                self.program.vars.push(Var {
                    name: name.to_owned(),
                    ty,
                    init: None,
                    kind: storage,
                    at: at(decl),
                });
                let id = self.program.vars.len() - 1;
                if storage.is_global() {
                    self.program.globals.push(id);
                }
                if storage == VarKind::Global {
                    self.globals.insert(name.to_owned(), id);
                }
                id
            }
        };
        self.vars.insert(text(decl, "id").to_owned(), id);
        if decl["init"].is_string()
            && let Some(init) = children(decl).last()
        {
            let init = self.expr(init)?;
            self.program.vars[id].init = Some(init);
        }
        Ok(id)
    }

    fn function(&mut self, decl: &Value) -> R<()> {
        let Some(body) = children(decl)
            .iter()
            .find(|child| kind(child) == "CompoundStmt")
        else {
            return Ok(());
        };
        let ret = match self.ty(decl)? {
            Type::Function { ret, .. } => *ret,
            _ => return unsupported("a function without a function type", decl),
        };
        let mut params = Vec::new();
        for param in children(decl) {
            if kind(param) == "ParmVarDecl" {
                params.push(self.var(param, VarKind::Param)?);
            }
        }
        self.labels.clear();
        collect_labels(body, &mut self.labels);
        let body = self.block(body)?;
        self.program.functions.push(Function {
            name: text(decl, "name").to_owned(),
            ret,
            params,
            variadic: decl["variadic"] == true,
            body,
        });
        Ok(())
    }

    /// The statements of a compound statement, or the one statement.
    fn block(&mut self, node: &Value) -> R<Vec<Stmt>> {
        let mut out = Vec::new();
        if kind(node) == "CompoundStmt" {
            for child in children(node) {
                self.stmt(child, &mut out)?;
            }
        } else {
            self.stmt(node, &mut out)?;
        }
        Ok(out)
    }

    fn stmt(&mut self, node: &Value, out: &mut Vec<Stmt>) -> R<()> {
        let inner = children(node);
        // Clang prints `{}` for a part that a statement leaves out.
        let part = |index: usize| inner.get(index).filter(|part| !kind(part).is_empty());
        match kind(node) {
            "CompoundStmt" => out.push(Stmt::Block(self.block(node)?)),
            "DeclStmt" => {
                for decl in inner {
                    match kind(decl) {
                        "VarDecl" => {
                            let storage = match text(decl, "storageClass") {
                                "static" => VarKind::Static(self.program.functions.len()),
                                "extern" => return unsupported("an `extern` local", decl),
                                _ => VarKind::Local,
                            };
                            out.push(Stmt::Decl(self.var(decl, storage)?));
                        }
                        "RecordDecl" => self.record(decl, true)?,
                        "TypedefDecl" => self.typedef(decl)?,
                        "EnumDecl" => self.enumeration(decl)?,
                        other => return unsupported(format!("Clang's `{other}` in a block"), decl),
                    }
                }
            }
            "IfStmt" => {
                let (Some(cond), Some(then)) = (part(0), part(1)) else {
                    return unsupported("an `if` without its parts", node);
                };
                let other = match part(2) {
                    Some(other) => self.block(other)?,
                    None => Vec::new(),
                };
                out.push(Stmt::If(self.expr(cond)?, self.block(then)?, other));
            }
            "WhileStmt" | "DoStmt" => {
                let (Some(first), Some(second)) = (part(0), part(1)) else {
                    return unsupported("a loop without its parts", node);
                };
                out.push(match kind(node) {
                    "WhileStmt" => Stmt::While(self.expr(first)?, self.block(second)?),
                    _ => Stmt::DoWhile(self.block(first)?, self.expr(second)?),
                });
            }
            "ForStmt" => {
                let Some(body) = part(4) else {
                    return unsupported("a `for` without a body", node);
                };
                out.push(Stmt::For {
                    init: part(0)
                        .map(|init| self.block(init))
                        .transpose()?
                        .unwrap_or_default(),
                    cond: part(2).map(|cond| self.expr(cond)).transpose()?,
                    step: part(3).map(|step| self.expr(step)).transpose()?,
                    body: self.block(body)?,
                });
            }
            "SwitchStmt" => {
                let (Some(value), Some(body)) = (part(0), part(1)) else {
                    return unsupported("a `switch` without its parts", node);
                };
                out.push(Stmt::Switch(self.expr(value)?, self.block(body)?));
            }
            "CaseStmt" => {
                let [value, body] = inner else {
                    return unsupported("a range of `case` values", node);
                };
                // Clang prints the value of some constants only.
                let value = match text(value, "value").parse() {
                    Ok(value) => value,
                    Err(_) => match self.expr(value)?.constant(&self.program) {
                        Some(value) => value,
                        None => return unsupported("a `case` without a constant", node),
                    },
                };
                out.push(Stmt::Case(value));
                self.stmt(body, out)?;
            }
            "DefaultStmt" => {
                out.push(Stmt::Default);
                if let Some(body) = part(0) {
                    self.stmt(body, out)?;
                }
            }
            "BreakStmt" => out.push(Stmt::Break),
            "ContinueStmt" => out.push(Stmt::Continue),
            "ReturnStmt" => out.push(Stmt::Return(
                part(0).map(|value| self.expr(value)).transpose()?,
            )),
            "NullStmt" => {}
            "GotoStmt" => match self.labels.get(text(node, "targetLabelDeclId")) {
                Some(label) => out.push(Stmt::Goto(label.clone())),
                None => return unsupported("a `goto` without its label", node),
            },
            "LabelStmt" => {
                out.push(Stmt::Label(text(node, "name").to_owned()));
                if let Some(body) = part(0) {
                    self.stmt(body, out)?;
                }
            }
            _ => out.push(Stmt::Expr(self.expr(node)?)),
        }
        Ok(())
    }

    fn expr(&mut self, node: &Value) -> R<Expr> {
        let inner = children(node);
        let child = |index: usize| match inner.get(index) {
            Some(child) => Ok(child),
            None => unsupported(format!("Clang's `{}` without its parts", kind(node)), node),
        };
        let boxed = |reader: &mut Self, index: usize| Ok(Box::new(reader.expr(child(index)?)?));
        let expr_kind = match kind(node) {
            "ParenExpr" | "ConstantExpr" => return self.expr(child(0)?),
            "ImplicitCastExpr" | "CStyleCastExpr" => match text(node, "castKind") {
                // The value stays as it is.
                "LValueToRValue" | "NoOp" | "FunctionToPointerDecay" | "BuiltinFnToFnPtr" => {
                    return self.expr(child(0)?);
                }
                "ArrayToPointerDecay" => ExprKind::Decay(boxed(self, 0)?),
                "NullToPointer" => ExprKind::Null,
                "BitCast" | "IntegralCast" | "IntegralToFloating" | "FloatingToIntegral"
                | "FloatingCast" | "IntegralToBoolean" | "FloatingToBoolean"
                | "PointerToBoolean" | "PointerToIntegral" | "IntegralToPointer" | "ToVoid" => {
                    ExprKind::Cast(boxed(self, 0)?)
                }
                other => return unsupported(format!("Clang's cast `{other}`"), node),
            },
            "IntegerLiteral" => match text(node, "value").parse() {
                Ok(value) => ExprKind::Int(value),
                Err(_) => return unsupported("an integer literal Clang did not print", node),
            },
            "CharacterLiteral" => match node["value"].as_u64().map(u8::try_from) {
                Some(Ok(value)) => ExprKind::Char(value),
                _ => ExprKind::Int(node["value"].as_i64().unwrap_or_default().into()),
            },
            "FloatingLiteral" => match text(node, "value").parse() {
                Ok(value) => ExprKind::Float(value),
                Err(_) => return unsupported("a floating literal Clang did not print", node),
            },
            "StringLiteral" => match unescape(text(node, "value")) {
                Some(bytes) => ExprKind::Str(bytes),
                None => return unsupported("a string literal with a prefix", node),
            },
            "DeclRefExpr" => {
                let decl = &node["referencedDecl"];
                let (id, name) = (text(decl, "id"), text(decl, "name"));
                match kind(decl) {
                    // Its type may be one that only a header needs.
                    "FunctionDecl" => {
                        return Ok(Expr {
                            ty: self.ty(node).unwrap_or(Type::Void),
                            kind: ExprKind::Function(name.to_owned()),
                            at: at(node),
                        });
                    }
                    "EnumConstantDecl" => match self.enumerators.get(id) {
                        Some(&index) => ExprKind::Enumerator(index),
                        None => return unsupported(format!("the enumerator `{name}`"), node),
                    },
                    _ => match self.vars.get(id) {
                        Some(&var) => ExprKind::Var(var),
                        None => ExprKind::Extern(name.to_owned()),
                    },
                }
            }
            "UnaryOperator" => {
                let operand = boxed(self, 0)?;
                match text(node, "opcode") {
                    "+" | "__extension__" => return Ok(*operand),
                    "-" => ExprKind::Unary(UnOp::Neg, operand),
                    "~" => ExprKind::Unary(UnOp::BitNot, operand),
                    "!" => ExprKind::Unary(UnOp::Not, operand),
                    "*" => ExprKind::Deref(operand),
                    "&" => ExprKind::AddrOf(operand),
                    op @ ("++" | "--") => ExprKind::IncDec {
                        target: operand,
                        increment: op == "++",
                        prefix: node["isPostfix"] != true,
                    },
                    other => return unsupported(format!("the operator `{other}`"), node),
                }
            }
            "BinaryOperator" => {
                let (left, right) = (boxed(self, 0)?, boxed(self, 1)?);
                match text(node, "opcode") {
                    "=" => ExprKind::Assign {
                        op: None,
                        target: left,
                        value: right,
                    },
                    "," => ExprKind::Comma(left, right),
                    op => match binary(op) {
                        Some(op) => ExprKind::Binary(op, left, right),
                        None => return unsupported(format!("the operator `{op}`"), node),
                    },
                }
            }
            "CompoundAssignOperator" => {
                let op = text(node, "opcode");
                let Some(op) = op.strip_suffix('=').and_then(binary) else {
                    return unsupported(format!("the operator `{op}`"), node);
                };
                let compute = self.type_of(&node["computeResultType"], node)?;
                ExprKind::Assign {
                    op: Some((op, compute)),
                    target: boxed(self, 0)?,
                    value: boxed(self, 1)?,
                }
            }
            "ConditionalOperator" => {
                ExprKind::Cond(boxed(self, 0)?, boxed(self, 1)?, boxed(self, 2)?)
            }
            "CallExpr" => {
                let callee = boxed(self, 0)?;
                let args = inner[1..]
                    .iter()
                    .map(|arg| self.expr(arg))
                    .collect::<R<_>>()?;
                ExprKind::Call(callee, args)
            }
            "MemberExpr" => {
                let Some(&(record, field)) = self.fields.get(text(node, "referencedMemberDecl"))
                else {
                    return unsupported("a member of a record from a header", node);
                };
                ExprKind::Member {
                    base: boxed(self, 0)?,
                    record,
                    field,
                    arrow: node["isArrow"] == true,
                }
            }
            "ArraySubscriptExpr" => {
                let (left, right) = (boxed(self, 0)?, boxed(self, 1)?);
                // C also allows `index[base]`.
                if left.ty.is_pointer() {
                    ExprKind::Index(left, right)
                } else {
                    ExprKind::Index(right, left)
                }
            }
            "UnaryExprOrTypeTraitExpr" if text(node, "name") == "sizeof" => {
                ExprKind::SizeOf(match node.get("argType") {
                    Some(ty) => self.type_of(ty, node)?,
                    None => self.ty(child(0)?)?,
                })
            }
            "InitListExpr" => {
                // With a filler, Clang prints it first and the items after it.
                let items = match node["array_filler"].as_array() {
                    Some(filled) => filled.get(1..).unwrap_or_default(),
                    None => inner,
                };
                if node.get("field").is_some() {
                    return unsupported("the initialiser of a union", node);
                }
                ExprKind::InitList(items.iter().map(|item| self.expr(item)).collect::<R<_>>()?)
            }
            "ImplicitValueInitExpr" => ExprKind::Zero,
            other => return unsupported(format!("Clang's `{other}`"), node),
        };
        Ok(Expr {
            ty: self.ty(node)?,
            kind: expr_kind,
            at: at(node),
        })
    }

    /// The type of a declaration or an expression.
    fn ty(&mut self, node: &Value) -> R<Type> {
        self.type_of(&node["type"], node)
    }

    fn type_of(&mut self, ty: &Value, node: &Value) -> R<Type> {
        let spelled = text(ty, "qualType");
        let mut read = self.type_str(spelled);
        if read.is_err()
            && let Some(desugared) = ty["desugaredQualType"].as_str()
        {
            read = self.type_str(desugared);
        }
        read.or_else(|what| unsupported(what, node))
    }

    /// The type that Clang prints as `spelled`, such as `const char *[4]`.
    fn type_str(&mut self, spelled: &str) -> Result<Type, String> {
        if let Some(known) = self.types.get(spelled) {
            return known.clone();
        }
        let read = match tokens(spelled) {
            Some(tokens) => self.full_type(&tokens),
            None => Err(String::new()),
        }
        .map_err(|_| format!("the type `{spelled}`"));
        self.types.insert(spelled.to_owned(), read.clone());
        read
    }

    /// Specifiers and an abstract declarator.
    fn full_type(&mut self, tokens: &[Token]) -> Result<Type, String> {
        let mut words = Vec::new();
        let mut base = None;
        let mut used = 0;
        while let Some(Token::Word(word)) = tokens.get(used) {
            match word.as_str() {
                word if is_qualifier(word) => {}
                "struct" | "union" | "enum" if base.is_none() && words.is_empty() => {
                    let Some(Token::Word(name)) = tokens.get(used + 1) else {
                        return Err(String::new());
                    };
                    base = Some(match word.as_str() {
                        "enum" => match self.enums.get(&format!("enum {name}")) {
                            Some(ty) => ty.clone(),
                            None => return Err(String::new()),
                        },
                        tag => match self.tags.get(&format!("{tag} {name}")) {
                            Some(&id) => Type::Record(id),
                            None => return Err(String::new()),
                        },
                    });
                    used += 1;
                }
                "void" | "char" | "short" | "int" | "long" | "signed" | "unsigned" | "float"
                | "double" | "_Bool" | "bool"
                    if base.is_none() =>
                {
                    words.push(word.as_str());
                }
                name if base.is_none() && words.is_empty() => {
                    base = Some(self.typedefs.get(name).cloned().ok_or(String::new())??);
                }
                _ => break,
            }
            used += 1;
        }
        let base = match base {
            Some(base) => base,
            None => builtin(&words).ok_or(String::new())?,
        };
        self.declarator(&tokens[used..], base)
    }

    /// Applies an abstract declarator such as `*`, `(*)[3]` or `(int)` to
    /// the type before it.
    fn declarator(&mut self, tokens: &[Token], mut ty: Type) -> Result<Type, String> {
        let mut used = 0;
        loop {
            match tokens.get(used) {
                Some(Token::Punct('*')) => ty = Type::Pointer(Box::new(ty)),
                Some(Token::Word(word)) if is_qualifier(word) => {}
                _ => break,
            }
            used += 1;
        }
        let rest = &tokens[used..];
        // `(*...)` groups a declarator; `(int)` lists parameters.
        if let [Token::Punct('('), Token::Punct('*' | '('), ..] = rest {
            let close = matching(rest, 0)?;
            let ty = self.suffixes(&rest[close + 1..], ty)?;
            return self.declarator(&rest[1..close], ty);
        }
        self.suffixes(rest, ty)
    }

    /// Applies array bounds and parameter lists.
    fn suffixes(&mut self, tokens: &[Token], mut ty: Type) -> Result<Type, String> {
        enum Suffix {
            Array(Option<u64>),
            Function(Vec<Type>, bool),
        }
        let mut suffixes = Vec::new();
        let mut at = 0;
        while at < tokens.len() {
            let close = matching(tokens, at)?;
            let inside = &tokens[at + 1..close];
            suffixes.push(match tokens[at] {
                Token::Punct('[') => Suffix::Array(match inside {
                    [Token::Number(length)] => Some(*length),
                    _ => None,
                }),
                Token::Punct('(') => {
                    let mut params = Vec::new();
                    let mut variadic = false;
                    for param in inside.split(|token| *token == Token::Punct(',')) {
                        match param {
                            [] => {}
                            [Token::Word(word)] if word == "void" => {}
                            [Token::Ellipsis] => variadic = true,
                            _ => params.push(self.full_type(param)?),
                        }
                    }
                    Suffix::Function(params, variadic)
                }
                _ => return Err(String::new()),
            });
            at = close + 1;
        }
        // `int[2][3]` is two arrays of three.
        for suffix in suffixes.into_iter().rev() {
            ty = match suffix {
                Suffix::Array(length) => Type::Array(Box::new(ty), length),
                Suffix::Function(params, variadic) => Type::Function {
                    ret: Box::new(ty),
                    params,
                    variadic,
                },
            };
        }
        Ok(ty)
    }
}

fn collect_labels(node: &Value, labels: &mut HashMap<String, String>) {
    if kind(node) == "LabelStmt" {
        labels.insert(
            text(node, "declId").to_owned(),
            text(node, "name").to_owned(),
        );
    }
    for child in children(node) {
        collect_labels(child, labels);
    }
}

fn binary(op: &str) -> Option<BinOp> {
    Some(match op {
        "+" => BinOp::Add,
        "-" => BinOp::Sub,
        "*" => BinOp::Mul,
        "/" => BinOp::Div,
        "%" => BinOp::Rem,
        "<<" => BinOp::Shl,
        ">>" => BinOp::Shr,
        "&" => BinOp::BitAnd,
        "|" => BinOp::BitOr,
        "^" => BinOp::BitXor,
        "<" => BinOp::Lt,
        ">" => BinOp::Gt,
        "<=" => BinOp::Le,
        ">=" => BinOp::Ge,
        "==" => BinOp::Eq,
        "!=" => BinOp::Ne,
        "&&" => BinOp::And,
        "||" => BinOp::Or,
        _ => return None,
    })
}

#[derive(Debug, PartialEq)]
enum Token {
    Word(String),
    Number(u64),
    Punct(char),
    Ellipsis,
}

/// The tokens of a type as Clang prints it, or `None` for one it prints
/// with a description, as for a record without a name.
fn tokens(spelled: &str) -> Option<Vec<Token>> {
    let mut tokens = Vec::new();
    let mut rest = spelled;
    while let Some(first) = rest.chars().next() {
        let length = match first {
            ' ' => 1,
            '*' | '(' | ')' | '[' | ']' | ',' => {
                tokens.push(Token::Punct(first));
                1
            }
            '.' if rest.starts_with("...") => {
                tokens.push(Token::Ellipsis);
                3
            }
            '0'..='9' => {
                let length = rest
                    .find(|c: char| !c.is_ascii_digit())
                    .unwrap_or(rest.len());
                tokens.push(Token::Number(rest[..length].parse().ok()?));
                length
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let length = rest
                    .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .unwrap_or(rest.len());
                tokens.push(Token::Word(rest[..length].to_owned()));
                length
            }
            _ => return None,
        };
        rest = &rest[length..];
    }
    Some(tokens)
}

/// The index of the bracket that closes the one at `open`.
fn matching(tokens: &[Token], open: usize) -> Result<usize, String> {
    let mut depth = 0;
    for (index, token) in tokens.iter().enumerate().skip(open) {
        match token {
            Token::Punct('(' | '[') => depth += 1,
            Token::Punct(')' | ']') => {
                depth -= 1;
                if depth == 0 {
                    return Ok(index);
                }
            }
            _ => {}
        }
    }
    Err(String::new())
}

fn is_qualifier(word: &str) -> bool {
    matches!(word, "const" | "volatile" | "restrict" | "__restrict")
}

/// The type that the words of a built-in type name, in any order.
fn builtin(words: &[&str]) -> Option<Type> {
    let has = |word| words.contains(&word);
    let signed = !has("unsigned");
    let int = |bits| Some(Type::Int { bits, signed });
    if words.is_empty() {
        None
    } else if has("void") {
        Some(Type::Void)
    } else if has("_Bool") || has("bool") {
        Some(Type::Bool)
    } else if has("float") {
        Some(Type::Float { bits: 32 })
    } else if has("double") {
        // `long double` has no Rust type.
        (!has("long")).then_some(Type::Float { bits: 64 })
    } else if has("char") {
        if has("signed") || has("unsigned") {
            int(8)
        } else {
            Some(Type::Char)
        }
    } else if has("short") {
        int(16)
    } else if has("long") {
        int(64)
    } else {
        int(32)
    }
}

/// The bytes of a string literal as Clang prints it, with its quotes and
/// escapes.
fn unescape(literal: &str) -> Option<Vec<u8>> {
    let body = literal.strip_prefix('"')?.strip_suffix('"')?;
    let mut bytes = Vec::new();
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            bytes.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
            continue;
        }
        let escaped = chars.next()?;
        // The value of up to `most` digits; C keeps the low byte of more.
        let mut digits = |radix: u32, first: u32, most: usize| {
            let mut value = first;
            let mut count = 1;
            while count < most
                && let Some(digit) = chars.peek().and_then(|c| c.to_digit(radix))
            {
                value = value.wrapping_mul(radix).wrapping_add(digit);
                chars.next();
                count += 1;
            }
            value as u8
        };
        bytes.push(match escaped {
            '0'..='7' => digits(8, escaped.to_digit(8)?, 3),
            'x' => digits(16, 0, usize::MAX),
            'n' => b'\n',
            't' => b'\t',
            'r' => b'\r',
            'a' => 7,
            'b' => 8,
            'f' => 12,
            'v' => 11,
            'e' => 27,
            '\\' | '\'' | '"' | '?' => escaped as u8,
            _ => return None,
        });
    }
    Some(bytes)
}
