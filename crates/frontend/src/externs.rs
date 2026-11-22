//! Signature extraction for `import x` resolving to `x.rs` or `x.c`.
//!
//! Only functions whose parameter and return types can cross the boundary
//! without guessing (integers, floats, bool, strings, unit) are exported;
//! everything else is listed as skipped with a reason.

use pyfast_ir::{ExtTy, ExternFn, ExternModule, Lang};
use std::path::Path;

pub fn load_rust(name: &str, path: &Path) -> Result<ExternModule, String> {
    let src = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let file = syn::parse_file(&src).map_err(|e| format!("{}: Rust parse error: {e}", path.display()))?;
    let mut funcs = Vec::new();
    let mut skipped = Vec::new();
    for item in &file.items {
        let syn::Item::Fn(f) = item else { continue };
        if !matches!(f.vis, syn::Visibility::Public(_)) {
            continue;
        }
        let fname = f.sig.ident.to_string();
        match rust_sig(&f.sig) {
            Ok(ef) => funcs.push(ExternFn { name: fname, ..ef }),
            Err(why) => skipped.push((fname, why)),
        }
    }
    Ok(ExternModule { name: name.into(), lang: Lang::Rust, path: abs(path), header: None, funcs, skipped })
}

fn rust_sig(sig: &syn::Signature) -> Result<ExternFn, String> {
    if !sig.generics.params.is_empty() {
        return Err("generic functions cannot be called from Python source".into());
    }
    if sig.asyncness.is_some() || sig.unsafety.is_some() || sig.variadic.is_some() {
        return Err("async, unsafe and variadic functions are not supported".into());
    }
    let mut params = Vec::new();
    for input in &sig.inputs {
        match input {
            syn::FnArg::Typed(pt) => params.push(rust_ty(&pt.ty, true)?),
            syn::FnArg::Receiver(_) => return Err("methods are not supported".into()),
        }
    }
    let (ret, fallible) = match &sig.output {
        syn::ReturnType::Default => (ExtTy::Unit, false),
        syn::ReturnType::Type(_, t) => match result_inner(t) {
            Some(inner) => (rust_ty(inner, false)?, true),
            None => (rust_ty(t, false)?, false),
        },
    };
    Ok(ExternFn { name: String::new(), params, ret, fallible })
}

/// `Result<T, E>` → `T`
fn result_inner(t: &syn::Type) -> Option<&syn::Type> {
    let syn::Type::Path(p) = t else { return None };
    let seg = p.path.segments.last()?;
    if seg.ident != "Result" {
        return None;
    }
    let syn::PathArguments::AngleBracketed(a) = &seg.arguments else { return None };
    match a.args.first()? {
        syn::GenericArgument::Type(t) => Some(t),
        _ => None,
    }
}

fn rust_ty(t: &syn::Type, param: bool) -> Result<ExtTy, String> {
    match t {
        syn::Type::Tuple(tt) if tt.elems.is_empty() => Ok(ExtTy::Unit),
        syn::Type::Reference(r) if r.mutability.is_none() => match &*r.elem {
            syn::Type::Path(p) if p.path.is_ident("str") && param => Ok(ExtTy::StrRef),
            _ => Err("only `&str` references are supported (as parameters)".into()),
        },
        syn::Type::Path(p) if p.qself.is_none() => {
            let id = p.path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default();
            Ok(match id.as_str() {
                "i8" | "i16" | "i32" | "i64" | "isize" | "u8" | "u16" | "u32" | "u64" | "usize" => ExtTy::Int(id),
                "f64" => ExtTy::F64,
                "f32" => ExtTy::F32,
                "bool" => ExtTy::Bool,
                "String" => ExtTy::OwnedString,
                other => return Err(format!("type `{other}` cannot cross the Python boundary yet")),
            })
        }
        _ => Err("unsupported parameter/return type".into()),
    }
}

/// Load a C module: signatures come from `x.h` next to `x.c` when present,
/// otherwise from the non-static function definitions in `x.c`.
pub fn load_c(name: &str, path: &Path) -> Result<ExternModule, String> {
    let header = path.with_extension("h");
    let (sig_src, header) = if header.exists() { (header.clone(), Some(abs(&header))) } else { (path.to_path_buf(), None) };
    let src = std::fs::read_to_string(&sig_src).map_err(|e| format!("{}: {e}", sig_src.display()))?;
    let mut funcs = Vec::new();
    let mut skipped = Vec::new();
    for head in c_function_heads(&src) {
        match c_sig(&head) {
            Ok(Some(f)) => {
                if !funcs.iter().any(|g: &ExternFn| g.name == f.name) {
                    funcs.push(f)
                }
            }
            Ok(None) => {}
            Err((n, why)) => skipped.push((n, why)),
        }
    }
    Ok(ExternModule { name: name.into(), lang: Lang::C, path: abs(path), header, funcs, skipped })
}

fn abs(p: &Path) -> String {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf()).display().to_string()
}

/// Strip comments and preprocessor lines, then return every top-level
/// `... name(params)` head that is followed by `;` or a body.
fn c_function_heads(src: &str) -> Vec<String> {
    let mut clean = String::new();
    let mut chars = src.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut prev = ' ';
                for d in chars.by_ref() {
                    if prev == '*' && d == '/' {
                        break;
                    }
                    prev = d;
                }
                clean.push(' ');
            }
            '/' if chars.peek() == Some(&'/') => {
                for d in chars.by_ref() {
                    if d == '\n' {
                        break;
                    }
                }
                clean.push('\n');
            }
            '"' => {
                // skip string literals
                let mut esc = false;
                for d in chars.by_ref() {
                    if !esc && d == '"' {
                        break;
                    }
                    esc = d == '\\' && !esc;
                }
                clean.push_str("\"\"");
            }
            _ => clean.push(c),
        }
    }
    let clean: String = clean.lines().filter(|l| !l.trim_start().starts_with('#')).collect::<Vec<_>>().join("\n");
    let mut heads = Vec::new();
    let mut cur = String::new();
    let mut depth = 0;
    for c in clean.chars() {
        match c {
            '{' => {
                if depth == 0 && cur.contains('(') && !cur.contains('=') {
                    heads.push(cur.trim().to_string());
                }
                depth += 1;
                cur.clear();
            }
            '}' => {
                depth -= 1;
                cur.clear();
            }
            ';' if depth == 0 => {
                if cur.contains('(') && !cur.contains('=') {
                    heads.push(cur.trim().to_string());
                }
                cur.clear();
            }
            _ if depth == 0 => cur.push(c),
            _ => {}
        }
    }
    heads
}

fn tokens(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in s.chars() {
        if c.is_alphanumeric() || c == '_' {
            cur.push(c);
        } else {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            if c == '*' {
                out.push("*".into());
            }
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

const C_TYPE_WORDS: &[&str] = &[
    "const", "volatile", "unsigned", "signed", "int", "long", "short", "char", "double", "float", "void", "bool", "_Bool",
    "int8_t", "int16_t", "int32_t", "int64_t", "uint8_t", "uint16_t", "uint32_t", "uint64_t", "size_t", "ssize_t", "*",
];

fn c_type(toks: &[String], param: bool) -> Result<ExtTy, String> {
    let t: Vec<&str> = toks.iter().map(|s| s.as_str()).filter(|t| *t != "const" && *t != "volatile").collect();
    let joined = t.join(" ");
    let c = |n: &str| ExtTy::Int(format!("std::ffi::{n}"));
    Ok(match joined.as_str() {
        "int" | "signed" | "signed int" => c("c_int"),
        "unsigned" | "unsigned int" => c("c_uint"),
        "long" | "long int" | "signed long" => c("c_long"),
        "unsigned long" | "unsigned long int" => c("c_ulong"),
        "long long" | "long long int" => c("c_longlong"),
        "unsigned long long" => c("c_ulonglong"),
        "short" | "short int" => c("c_short"),
        "unsigned short" => c("c_ushort"),
        "int8_t" => ExtTy::Int("i8".into()),
        "int16_t" => ExtTy::Int("i16".into()),
        "int32_t" => ExtTy::Int("i32".into()),
        "int64_t" => ExtTy::Int("i64".into()),
        "uint8_t" => ExtTy::Int("u8".into()),
        "uint16_t" => ExtTy::Int("u16".into()),
        "uint32_t" => ExtTy::Int("u32".into()),
        "uint64_t" => ExtTy::Int("u64".into()),
        "size_t" => ExtTy::Int("usize".into()),
        "ssize_t" => ExtTy::Int("isize".into()),
        "double" => ExtTy::F64,
        "float" => ExtTy::F32,
        "bool" | "_Bool" => ExtTy::Bool,
        "void" => ExtTy::Unit,
        "char *" => {
            let is_const = toks.iter().any(|t| t == "const");
            if param || is_const {
                ExtTy::CStr
            } else {
                return Err("returning non-const `char *` has unknown ownership; return `const char *`".into());
            }
        }
        other => return Err(format!("C type `{other}` cannot cross the Python boundary yet")),
    })
}

/// Parse `ret name(params)`; `Ok(None)` for heads that are not plain
/// exported functions (static functions, function pointers, macros).
fn c_sig(head: &str) -> Result<Option<ExternFn>, (String, String)> {
    let Some(open) = head.find('(') else { return Ok(None) };
    let Some(close) = head.rfind(')') else { return Ok(None) };
    let before = tokens(&head[..open]);
    if before.iter().any(|t| t == "static" || t == "typedef") || head[..open].contains('(') {
        return Ok(None);
    }
    let Some(name) = before.last().cloned() else { return Ok(None) };
    if C_TYPE_WORDS.contains(&name.as_str()) || !name.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_') {
        return Ok(None);
    }
    let ret_toks: Vec<String> = before[..before.len() - 1].iter().filter(|t| *t != "extern" && *t != "inline").cloned().collect();
    if ret_toks.is_empty() {
        return Ok(None);
    }
    let ret = c_type(&ret_toks, false).map_err(|e| (name.clone(), e))?;
    let inner = head[open + 1..close].trim();
    let mut params = Vec::new();
    if !(inner.is_empty() || inner == "void") {
        for p in inner.split(',') {
            let mut toks = tokens(p);
            if p.contains("...") {
                return Err((name, "variadic C functions are not supported".into()));
            }
            // Drop the parameter name if present.
            if toks.len() > 1 && !C_TYPE_WORDS.contains(&toks.last().unwrap().as_str()) {
                toks.pop();
            }
            params.push(c_type(&toks, true).map_err(|e| (name.clone(), e))?);
        }
    }
    Ok(Some(ExternFn { name, params, ret, fallible: false }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn c_prototypes() {
        let src = r#"
            #include <stdio.h>
            /* comment with fake(int x); */
            int add(int a, int b);
            double scale(double x, float k) { return x * k; }
            static int hidden(int x) { return x; }
            const char *greet(const char *who);
            char *leak(void);
            void nothing(void);
            int64_t big(uint32_t n, size_t m);
        "#;
        let heads = c_function_heads(src);
        let sigs: Vec<_> = heads.iter().map(|h| c_sig(h)).collect();
        let ok: Vec<String> = sigs.iter().filter_map(|s| s.as_ref().ok().cloned().flatten()).map(|f| f.name).collect();
        assert_eq!(ok, ["add", "scale", "greet", "nothing", "big"]);
        assert!(sigs.iter().any(|s| matches!(s, Err((n, _)) if n == "leak")));
    }

    #[test]
    fn rust_signatures() {
        let f: syn::File = syn::parse_str(
            "pub fn dot(a: f64, b: f64) -> f64 { a * b }
             pub fn name(s: &str) -> String { s.into() }
             pub fn checked(n: i32) -> Result<u64, String> { Ok(n as u64) }
             pub fn gen<T>(t: T) -> T { t }
             fn private() {}",
        )
        .unwrap();
        let sigs: Vec<_> = f
            .items
            .iter()
            .filter_map(|i| if let syn::Item::Fn(f) = i { Some(rust_sig(&f.sig)) } else { None })
            .collect();
        assert!(sigs[0].is_ok() && sigs[1].is_ok());
        assert!(sigs[2].as_ref().unwrap().fallible);
        assert!(sigs[3].is_err());
    }
}
