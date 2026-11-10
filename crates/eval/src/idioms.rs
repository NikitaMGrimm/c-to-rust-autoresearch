//! Idiomaticity counts of a translation.

use std::collections::BTreeMap;

/// Counts constructs that show how idiomatic a translation is. Counting the
/// syntax tree ignores comments and strings that mention them.
pub(crate) fn idiom_counts(source: &str) -> Option<BTreeMap<&'static str, u32>> {
    use syn::visit::Visit;

    struct Counter(BTreeMap<&'static str, u32>);
    impl Counter {
        fn add(&mut self, name: &'static str) {
            *self.0.entry(name).or_default() += 1;
        }
    }
    impl Counter {
        /// A function or method named like the standard library's that give
        /// up freeing memory, or that turn scalars into bytes and back.
        fn call(&mut self, name: &syn::Ident) {
            let name = name.to_string();
            if name == "leak" || name == "forget" {
                self.add("leaks");
            }
            let bytes = ["le_bytes", "be_bytes", "ne_bytes"];
            if let Some(order) = name.strip_prefix("from_").or(name.strip_prefix("to_"))
                && bytes.contains(&order)
            {
                self.add("byte_casts");
            }
        }
    }
    impl<'ast> Visit<'ast> for Counter {
        fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
            if call.method == "clone" && call.args.is_empty() {
                self.add("clones");
            }
            self.call(&call.method);
            syn::visit::visit_expr_method_call(self, call);
        }
        fn visit_expr_path(&mut self, path: &'ast syn::ExprPath) {
            let segments = &path.path.segments;
            if let Some(last) = segments.last() {
                self.call(&last.ident);
            }
            if segments
                .iter()
                .any(|segment| segment.ident == "ManuallyDrop")
            {
                self.add("leaks");
            }
            syn::visit::visit_expr_path(self, path);
        }
        fn visit_type_ptr(&mut self, ptr: &'ast syn::TypePtr) {
            self.add("raw_pointers");
            syn::visit::visit_type_ptr(self, ptr);
        }
        fn visit_expr_raw_addr(&mut self, expr: &'ast syn::ExprRawAddr) {
            self.add("raw_pointers");
            syn::visit::visit_expr_raw_addr(self, expr);
        }
        fn visit_type_path(&mut self, ty: &'ast syn::TypePath) {
            if wraps(&ty.path, "Rc", "RefCell") {
                self.add("rc_refcell");
            }
            if ty
                .path
                .segments
                .last()
                .is_some_and(|last| last.ident == "ManuallyDrop")
            {
                self.add("leaks");
            }
            syn::visit::visit_type_path(self, ty);
        }
        fn visit_attribute(&mut self, attr: &'ast syn::Attribute) {
            // Lint counts are only meaningful next to the lints switched off.
            if attr.path().is_ident("allow") || attr.path().is_ident("expect") {
                self.add("lint_allows");
            }
            syn::visit::visit_attribute(self, attr);
        }
    }

    fn wraps(path: &syn::Path, outer: &str, inner: &str) -> bool {
        let Some(last) = path.segments.last().filter(|last| last.ident == outer) else {
            return false;
        };
        let syn::PathArguments::AngleBracketed(args) = &last.arguments else {
            return false;
        };
        args.args.iter().any(|arg| {
            matches!(arg, syn::GenericArgument::Type(syn::Type::Path(ty))
                if ty.path.segments.last().is_some_and(|segment| segment.ident == inner))
        })
    }

    let file = syn::parse_file(source).ok()?;
    let names = [
        "clones",
        "raw_pointers",
        "rc_refcell",
        "lint_allows",
        "leaks",
        "byte_casts",
    ];
    let mut counter = Counter(names.into_iter().map(|name| (name, 0)).collect());
    counter.visit_file(&file);
    let lines = source
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();
    counter
        .0
        .insert("lines", u32::try_from(lines).unwrap_or(u32::MAX));
    Some(counter.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idiom_counts_come_from_code_not_comments() {
        let source = r#"#![allow(dead_code)]
use std::cell::RefCell;
use std::rc::Rc;
// x.clone() in a comment does not count
fn main() {
    let shared: Rc<RefCell<i32>> = Rc::new(RefCell::new(1));
    let other = shared.clone();
    let x = 1;
    let p: *const i32 = &raw const x;
    println!("{:?} {:?} clone()", other, p);
    let kept: &'static mut [u8] = Box::leak(vec![0u8; 4].into_boxed_slice());
    let word = u32::from_le_bytes([kept[0], kept[1], kept[2], kept[3]]);
    kept.copy_from_slice(&word.to_le_bytes());
    std::mem::forget(std::mem::ManuallyDrop::new(vec![1].leak()));
}
"#;
        let counts = idiom_counts(source).unwrap();
        assert_eq!(counts["clones"], 1);
        assert_eq!(counts["raw_pointers"], 2);
        assert_eq!(counts["rc_refcell"], 1);
        assert_eq!(counts["lint_allows"], 1);
        assert_eq!(counts["leaks"], 4);
        assert_eq!(counts["byte_casts"], 2);
        assert_eq!(counts["lines"], 15);
        assert_eq!(idiom_counts("fn main( {"), None);
    }
}
