//! Spells out the types that typedefs and enums name. Clang resolves a
//! typedef only at the top of a type, so a `const uint8_t *` stays as written,
//! and gives an enumeration constant only as a reference to its declaration.

use crate::translate::{inner, is_bool, kind, number_type};
use serde_json::{Value, json};
use std::collections::HashMap;

/// Returns `ast` with every typedef of a number or `bool` and every enum type
/// spelled as the type it is, in each type the tree gives, and every use of
/// an enumeration constant as the integer literal of its value, negated if
/// it is negative.
pub(crate) fn resolve(ast: &Value) -> Value {
    let mut names = HashMap::new();
    let mut enums = HashMap::new();
    let mut constants = HashMap::new();
    collect_enums(ast, &mut enums, &mut constants);
    for decl in inner(ast) {
        match kind(decl) {
            "EnumDecl" => {
                let tag = decl["name"].as_str().unwrap_or("");
                if let (false, Some(ty)) = (tag.is_empty(), enum_type(decl, &enums)) {
                    names.insert(format!("enum {tag}"), ty.to_string());
                }
            }
            "TypedefDecl" => {
                let Some(name) = decl["name"].as_str() else {
                    continue;
                };
                let ty = match aliased_enum(decl).and_then(|id| enums.get(id)) {
                    // Clang spells an unnamed enum by its typedef, as `enum name`.
                    Some(&ty) => {
                        names.insert(format!("enum {name}"), ty.to_string());
                        ty.to_string()
                    }
                    None => spell(type_of(&decl["type"]), &names),
                };
                if number_type(&ty).is_some() || is_bool(&ty) {
                    names.insert(name.to_string(), ty);
                }
            }
            _ => {}
        }
    }
    let mut ast = ast.clone();
    rewrite(&mut ast, &names, &constants);
    ast
}

fn type_of(ty: &Value) -> &str {
    ty["desugaredQualType"]
        .as_str()
        .or(ty["qualType"].as_str())
        .unwrap_or("")
}

/// Records the value of each enumeration constant by its id, and the integer
/// type of each enum by its id. A constant without a value is one more than
/// the one before it, the first 0.
fn collect_enums<'v>(
    node: &'v Value,
    enums: &mut HashMap<&'v str, &'static str>,
    constants: &mut HashMap<&'v str, i128>,
) {
    if kind(node) == "EnumDecl" {
        let mut next = 0;
        let mut values = Vec::new();
        for constant in inner(node).iter().filter(|n| kind(n) == "EnumConstantDecl") {
            let value = inner(constant)
                .first()
                .and_then(|init| init["value"].as_str()?.parse().ok())
                .unwrap_or(next);
            if let Some(id) = constant["id"].as_str() {
                constants.insert(id, value);
            }
            values.push(value);
            next = value + 1;
        }
        if let (Some(id), Some(ty)) = (node["id"].as_str(), integer_type(node, &values)) {
            enums.insert(id, ty);
        }
    }
    for child in inner(node) {
        collect_enums(child, enums, constants);
    }
}

/// The integer type of an enum with these values: the type it is declared
/// with, or else `unsigned int` without negative values and `int` with them,
/// as GCC and Clang choose. An enum needing a wider type has none here.
fn integer_type(decl: &Value, values: &[i128]) -> Option<&'static str> {
    if let Some(fixed) = decl["fixedUnderlyingType"]["qualType"].as_str() {
        return ["int", "unsigned int", "long", "unsigned long"]
            .into_iter()
            .find(|&ty| ty == fixed);
    }
    if values
        .iter()
        .all(|v| (0..=i128::from(u32::MAX)).contains(v))
    {
        Some("unsigned int")
    } else if values
        .iter()
        .all(|v| (i128::from(i32::MIN)..=i128::from(i32::MAX)).contains(v))
    {
        Some("int")
    } else {
        None
    }
}

fn enum_type<'a>(decl: &Value, enums: &HashMap<&str, &'a str>) -> Option<&'a str> {
    enums.get(decl["id"].as_str()?).copied()
}

/// The id of the enum that a typedef names, if it names one.
fn aliased_enum(decl: &Value) -> Option<&str> {
    let mut ty = inner(decl).first();
    while let Some(node) = ty.filter(|node| kind(node) == "ElaboratedType") {
        ty = inner(node).first();
    }
    ty.filter(|node| kind(node) == "EnumType")?["decl"]["id"].as_str()
}

/// `ty` with each name in `names` replaced by the type it stands for. The tag
/// of a struct or union is no typedef, and the name of an unnamed type, which
/// gives where it is, holds no type.
fn spell(ty: &str, names: &HashMap<String, String>) -> String {
    let word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut out = String::new();
    let mut rest = ty;
    while let Some(start) = rest.find(|c: char| c.is_ascii_alphabetic() || c == '_' || c == '(') {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        if rest.starts_with("(unnamed ") || rest.starts_with("(anonymous ") {
            let end = rest.find(')').map_or(rest.len(), |end| end + 1);
            out.push_str(&rest[..end]);
            rest = &rest[end..];
            continue;
        }
        if let Some(after) = rest.strip_prefix('(') {
            out.push('(');
            rest = after;
            continue;
        }
        let mut end = rest.find(|c: char| !word(c)).unwrap_or(rest.len());
        let name = &rest[..end];
        if matches!(name, "enum" | "struct" | "union")
            && let Some(tag) = rest[end..].strip_prefix(' ')
            && tag.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        {
            end += 1 + tag.find(|c: char| !word(c)).unwrap_or(tag.len());
        }
        let name = &rest[..end];
        out.push_str(names.get(name).map_or(name, String::as_str));
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// Spells the types of `node` and its children with `names`, and replaces
/// each reference to an enumeration constant with its value.
fn rewrite(node: &mut Value, names: &HashMap<String, String>, constants: &HashMap<&str, i128>) {
    let constant = (kind(node) == "DeclRefExpr")
        .then(|| node["referencedDecl"]["id"].as_str())
        .flatten()
        .and_then(|id| constants.get(id).copied());
    if let Some(value) = constant {
        let literal = json!({
            "kind": "IntegerLiteral",
            "type": node["type"].clone(),
            "valueCategory": "prvalue",
            "value": value.unsigned_abs().to_string(),
        });
        *node = if value < 0 {
            json!({
                "kind": "UnaryOperator",
                "type": node["type"].clone(),
                "valueCategory": "prvalue",
                "opcode": "-",
                "isPostfix": false,
                "inner": [literal],
            })
        } else {
            literal
        };
    }
    match node {
        Value::Object(fields) => {
            for (key, value) in fields.iter_mut() {
                match value {
                    Value::String(ty)
                        if matches!(key.as_str(), "qualType" | "desugaredQualType") =>
                    {
                        *ty = spell(ty, names);
                    }
                    _ => rewrite(value, names, constants),
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                rewrite(value, names, constants);
            }
        }
        _ => {}
    }
}
