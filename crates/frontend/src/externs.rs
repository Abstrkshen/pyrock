//! Signature extraction for `import x` resolving to `x.rs` or `x.c`.
//!
//! Only functions whose parameter and return types can cross the boundary
//! without guessing (integers, floats, bool, strings, unit) are exported;
//! everything else is listed as skipped with a reason.

use pyfast_ir::{ExtTy, ExternFn, ExternModule, ExternStruct, Lang, Pass};
use std::path::Path;

/// `// pyfast: <key> <value>` build settings in a Rust/C file.
pub(crate) fn directives(src: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in src.lines() {
        let l = line.trim();
        let body = l.strip_prefix("//!").or_else(|| l.strip_prefix("//")).or_else(|| l.strip_prefix("/*")).map(|b| b.trim().trim_end_matches("*/").trim());
        if let Some(rest) = body.and_then(|b| b.strip_prefix("pyfast:")) {
            if let Some((k, v)) = rest.trim().split_once(char::is_whitespace) {
                out.push((k.trim().to_string(), v.trim().to_string()));
            }
        }
    }
    out
}

pub fn load_rust(name: &str, path: &Path) -> Result<ExternModule, String> {
    let src = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let file = syn::parse_file(&src).map_err(|e| format!("{}: Rust parse error: {e}", path.display()))?;
    let mut funcs = Vec::new();
    let mut skipped = Vec::new();
    // Structs first: signatures may mention them.
    let mut structs: Vec<ExternStruct> = Vec::new();
    for item in &file.items {
        let syn::Item::Struct(st) = item else { continue };
        if !matches!(st.vis, syn::Visibility::Public(_)) || !st.generics.params.is_empty() {
            continue;
        }
        let derives = derive_names(&st.attrs);
        structs.push(ExternStruct { name: st.ident.to_string(), fields: vec![], debug: derives.iter().any(|d| d == "Debug"), clone: derives.iter().any(|d| d == "Clone") });
    }
    let names: Vec<String> = structs.iter().map(|s| s.name.clone()).collect();
    for item in &file.items {
        let syn::Item::Struct(st) = item else { continue };
        let Some(idx) = names.iter().position(|n| *n == st.ident.to_string()) else { continue };
        if let syn::Fields::Named(fs) = &st.fields {
            for f in &fs.named {
                if !matches!(f.vis, syn::Visibility::Public(_)) {
                    continue;
                }
                if let Ok(t) = rust_ty(&f.ty, false, &names, name) {
                    structs[idx].fields.push((f.ident.as_ref().unwrap().to_string(), t));
                }
            }
        }
    }
    for item in &file.items {
        match item {
            syn::Item::Fn(f) => {
                if !matches!(f.vis, syn::Visibility::Public(_)) {
                    continue;
                }
                let fname = f.sig.ident.to_string();
                match rust_sig(&f.sig, &names, name, None) {
                    Ok(ef) => funcs.push(ExternFn { name: fname, ..ef }),
                    Err(why) => skipped.push((fname, why)),
                }
            }
            syn::Item::Impl(im) if im.trait_.is_none() && im.generics.params.is_empty() => {
                let syn::Type::Path(tp) = &*im.self_ty else { continue };
                let sname = tp.path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default();
                let Some(owner) = names.iter().position(|n| *n == sname) else { continue };
                for it in &im.items {
                    let syn::ImplItem::Fn(m) = it else { continue };
                    if !matches!(m.vis, syn::Visibility::Public(_)) {
                        continue;
                    }
                    let fname = format!("{sname}::{}", m.sig.ident);
                    match rust_sig(&m.sig, &names, name, Some((owner, &sname))) {
                        Ok(ef) => funcs.push(ExternFn { name: fname, ..ef }),
                        Err(why) => skipped.push((fname, why)),
                    }
                }
            }
            _ => {}
        }
    }
    let deps: Vec<String> = directives(&src).into_iter().filter(|(k, _)| k == "dep" || k == "dependency").map(|(_, v)| v).collect();
    Ok(ExternModule { name: name.into(), lang: Lang::Rust, path: abs(path), header: None, funcs, skipped, structs, deps, link_libs: vec![], pkg_config: vec![] })
}

fn derive_names(attrs: &[syn::Attribute]) -> Vec<String> {
    let mut out = Vec::new();
    for a in attrs {
        if a.path().is_ident("derive") {
            let _ = a.parse_nested_meta(|m| {
                if let Some(id) = m.path.get_ident() {
                    out.push(id.to_string());
                }
                Ok(())
            });
        }
    }
    out
}

fn rust_sig(sig: &syn::Signature, structs: &[String], module: &str, owner: Option<(usize, &str)>) -> Result<ExternFn, String> {
    if !sig.generics.params.iter().all(|p| matches!(p, syn::GenericParam::Lifetime(_))) {
        return Err("generic functions cannot be called from Python source".into());
    }
    if sig.asyncness.is_some() || sig.unsafety.is_some() || sig.variadic.is_some() {
        return Err("async, unsafe and variadic functions are not supported".into());
    }
    let mut params = Vec::new();
    let mut recv = None;
    for input in &sig.inputs {
        match input {
            syn::FnArg::Typed(pt) => params.push(rust_ty(&self_to(&pt.ty, owner), true, structs, module)?),
            syn::FnArg::Receiver(r) => {
                recv = Some(match (&r.reference, r.mutability) {
                    (Some(_), Some(_)) => Pass::Mut,
                    (Some(_), None) => Pass::Ref,
                    (None, _) => Pass::Value,
                });
            }
        }
    }
    let (ret, fallible) = match &sig.output {
        syn::ReturnType::Default => (ExtTy::Unit, false),
        syn::ReturnType::Type(_, t) => {
            let t = self_to(t, owner);
            match result_inner(&t) {
                Some(inner) => (rust_ty(&self_to(inner, owner), false, structs, module)?, true),
                None => (rust_ty(&t, false, structs, module)?, false),
            }
        }
    };
    if let ExtTy::Struct { pass, .. } = &ret {
        if *pass != Pass::Value {
            return Err("returning a reference to a struct is not supported (return it by value)".into());
        }
    }
    Ok(ExternFn { name: String::new(), params, ret, fallible, owner: owner.map(|o| o.0), recv })
}

/// `Self` inside an `impl` block names the struct.
fn self_to(t: &syn::Type, owner: Option<(usize, &str)>) -> syn::Type {
    if let (syn::Type::Path(p), Some((_, n))) = (t, owner) {
        if p.path.is_ident("Self") {
            return syn::parse_str::<syn::Type>(n).unwrap();
        }
    }
    t.clone()
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

/// The generic arguments of `Vec<T>`, `Option<T>`, `HashMap<K, V>`.
fn generic_args(seg: &syn::PathSegment) -> Vec<&syn::Type> {
    match &seg.arguments {
        syn::PathArguments::AngleBracketed(a) => a.args.iter().filter_map(|g| if let syn::GenericArgument::Type(t) = g { Some(t) } else { None }).collect(),
        _ => vec![],
    }
}

fn struct_ty(structs: &[String], module: &str, name: &str, pass: Pass) -> Option<ExtTy> {
    let idx = structs.iter().position(|s| s == name)?;
    Some(ExtTy::Struct { ext: usize::MAX, idx, name: format!("E_{module}_{name}"), pass })
}

fn rust_ty(t: &syn::Type, param: bool, structs: &[String], module: &str) -> Result<ExtTy, String> {
    match t {
        syn::Type::Tuple(tt) if tt.elems.is_empty() => Ok(ExtTy::Unit),
        syn::Type::Tuple(tt) => Ok(ExtTy::Tuple(tt.elems.iter().map(|e| rust_ty(e, param, structs, module)).collect::<Result<_, _>>()?)),
        syn::Type::Reference(r) => {
            let mutable = r.mutability.is_some();
            match &*r.elem {
                syn::Type::Path(p) if p.path.is_ident("str") && param && !mutable => Ok(ExtTy::StrRef),
                syn::Type::Slice(sl) if param => {
                    let e = rust_ty(&sl.elem, param, structs, module)?;
                    if !mutable && e == ExtTy::Int("u8".into()) {
                        return Ok(ExtTy::Bytes(Pass::Ref));
                    }
                    Ok(if mutable { ExtTy::SliceMut(Box::new(e), false) } else { ExtTy::Slice(Box::new(e)) })
                }
                syn::Type::Path(p) if param && p.path.segments.last().is_some_and(|s| s.ident == "Vec") => {
                    let seg = p.path.segments.last().unwrap();
                    let inner = generic_args(seg).first().copied().ok_or("Vec without element type")?;
                    let e = rust_ty(inner, param, structs, module)?;
                    Ok(if mutable { ExtTy::SliceMut(Box::new(e), true) } else { ExtTy::Slice(Box::new(e)) })
                }
                syn::Type::Path(p) if param => {
                    let id = p.path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default();
                    struct_ty(structs, module, &id, if mutable { Pass::Mut } else { Pass::Ref }).ok_or_else(|| format!("references to `{id}` cannot cross the Python boundary"))
                }
                syn::Type::TraitObject(to) if param && !mutable => fn_trait(to.bounds.iter(), structs, module, true),
                _ => Err("unsupported reference type".into()),
            }
        }
        syn::Type::ImplTrait(it) if param => fn_trait(it.bounds.iter(), structs, module, false),
        syn::Type::Path(p) if p.qself.is_none() => {
            let seg = p.path.segments.last().ok_or("empty path")?;
            let id = seg.ident.to_string();
            let args = generic_args(seg);
            let arg = |i: usize| -> Result<ExtTy, String> { rust_ty(args.get(i).ok_or("missing type argument")?, param, structs, module) };
            Ok(match id.as_str() {
                "i8" | "i16" | "i32" | "i64" | "isize" | "u8" | "u16" | "u32" | "u64" | "usize" => ExtTy::Int(id),
                "f64" => ExtTy::F64,
                "f32" => ExtTy::F32,
                "bool" => ExtTy::Bool,
                "String" => ExtTy::OwnedString,
                "Vec" if args.first().is_some_and(|a| matches!(a, syn::Type::Path(p) if p.path.is_ident("u8"))) => ExtTy::Bytes(Pass::Value),
                "Vec" => ExtTy::Vec(Box::new(arg(0)?)),
                "Option" => ExtTy::Opt(Box::new(arg(0)?)),
                "HashMap" => ExtTy::Map(Box::new(arg(0)?), Box::new(arg(1)?), false),
                "BTreeMap" => ExtTy::Map(Box::new(arg(0)?), Box::new(arg(1)?), true),
                other => match struct_ty(structs, module, other, Pass::Value) {
                    Some(s) => s,
                    None => return Err(format!("type `{other}` cannot cross the Python boundary yet")),
                },
            })
        }
        _ => Err("unsupported parameter/return type".into()),
    }
}

/// `impl Fn(A, B) -> R` / `&dyn Fn(...)`.
fn fn_trait<'a>(bounds: impl Iterator<Item = &'a syn::TypeParamBound>, structs: &[String], module: &str, dyn_ref: bool) -> Result<ExtTy, String> {
    for b in bounds {
        let syn::TypeParamBound::Trait(tb) = b else { continue };
        let seg = tb.path.segments.last().ok_or("empty trait path")?;
        if !matches!(seg.ident.to_string().as_str(), "Fn" | "FnMut") {
            continue;
        }
        let syn::PathArguments::Parenthesized(pa) = &seg.arguments else { return Err("Fn without arguments".into()) };
        let params: Vec<ExtTy> = pa.inputs.iter().map(|t| rust_ty(t, false, structs, module)).collect::<Result<_, _>>()?;
        let ret = match &pa.output {
            syn::ReturnType::Default => ExtTy::Unit,
            syn::ReturnType::Type(_, t) => rust_ty(t, false, structs, module)?,
        };
        return Ok(ExtTy::Callback(params, Box::new(ret), dyn_ref));
    }
    Err("only `impl Fn(..)` / `&dyn Fn(..)` trait parameters are supported".into())
}

/// Load a C module: signatures come from `x.h` next to `x.c` when present,
/// otherwise from the non-static function definitions in `x.c`.
pub fn load_c(name: &str, path: &Path) -> Result<ExternModule, String> {
    let header = path.with_extension("h");
    let (sig_src, header) = if header.exists() { (header.clone(), Some(abs(&header))) } else { (path.to_path_buf(), None) };
    let src = std::fs::read_to_string(&sig_src).map_err(|e| format!("{}: {e}", sig_src.display()))?;
    let mut funcs = Vec::new();
    let mut skipped = Vec::new();
    // `typedef struct [Tag] { fields } Name;` (passed by value, like raylib's Color).
    let structs = c_structs(&src, name);
    let snames: Vec<String> = structs.iter().map(|s| s.name.clone()).collect();
    for (i, st) in structs.iter().enumerate() {
        funcs.push(ExternFn {
            name: format!("{}::__new__", st.name),
            params: st.fields.iter().map(|(_, t)| t.clone()).collect(),
            ret: ExtTy::Struct { ext: usize::MAX, idx: i, name: format!("E_{name}_{}", st.name), pass: Pass::Value },
            fallible: false,
            owner: Some(i),
            recv: None,
        });
    }
    for head in c_function_heads(&src) {
        match c_sig(&head, &snames, name) {
            Ok(Some(f)) => {
                if !funcs.iter().any(|g: &ExternFn| g.name == f.name) {
                    funcs.push(f)
                }
            }
            Ok(None) => {}
            Err((n, why)) => skipped.push((n, why)),
        }
    }
    let all_src = std::fs::read_to_string(path).unwrap_or_default();
    let dirs = directives(&all_src);
    let link_libs = dirs.iter().filter(|(k, _)| k == "link").flat_map(|(_, v)| v.split(',').map(|x| x.trim().to_string())).collect();
    let pkg_config = dirs.iter().filter(|(k, _)| k == "pkg-config").flat_map(|(_, v)| v.split(',').map(|x| x.trim().to_string())).collect();
    Ok(ExternModule { name: name.into(), lang: Lang::C, path: abs(path), header, funcs, skipped, structs, deps: vec![], link_libs, pkg_config })
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

/// Split a parameter list at commas outside parentheses.
fn split_top(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

/// `typedef struct [Tag] { T a; U b, c; } Name;`
fn c_structs(src: &str, module: &str) -> Vec<ExternStruct> {
    let mut out: Vec<ExternStruct> = Vec::new();
    let mut rest = src;
    while let Some(at) = rest.find("typedef struct") {
        let after = &rest[at + "typedef struct".len()..];
        let (Some(open), Some(close)) = (after.find('{'), after.find('}')) else { break };
        let Some(semi) = after[close..].find(';') else { break };
        let body = &after[open + 1..close];
        let sname = after[close + 1..close + semi].trim().to_string();
        rest = &after[close + semi..];
        if sname.is_empty() || !sname.chars().all(|c| c.is_alphanumeric() || c == '_') {
            continue;
        }
        let known: Vec<String> = out.iter().map(|s| s.name.clone()).collect();
        let mut fields = Vec::new();
        let mut ok = true;
        for decl in body.split(';').map(str::trim).filter(|d| !d.is_empty()) {
            let toks = tokens(decl);
            // `float x, y` -> type tokens, then names.
            let names_part: Vec<&str> = decl.split(',').collect();
            let first = tokens(names_part[0]);
            if first.len() < 2 {
                ok = false;
                break;
            }
            let ty_toks: Vec<String> = first[..first.len() - 1].to_vec();
            let Ok(t) = c_type_s(&ty_toks, false, &known, module) else {
                ok = false;
                break;
            };
            fields.push((first.last().unwrap().clone(), t.clone()));
            for extra in &names_part[1..] {
                let n = tokens(extra);
                if let Some(nm) = n.last() {
                    fields.push((nm.clone(), t.clone()));
                }
            }
            let _ = toks;
        }
        if ok {
            out.push(ExternStruct { name: sname, fields, debug: false, clone: true });
        }
    }
    out
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
    c_type_s(toks, param, &[], "")
}

fn c_type_s(toks: &[String], param: bool, structs: &[String], module: &str) -> Result<ExtTy, String> {
    let t: Vec<&str> = toks.iter().map(|s| s.as_str()).filter(|t| *t != "const" && *t != "volatile" && *t != "struct").collect();
    let joined = t.join(" ");
    if let Some(i) = structs.iter().position(|s| *s == joined) {
        return Ok(ExtTy::Struct { ext: usize::MAX, idx: i, name: format!("E_{module}_{joined}"), pass: Pass::Value });
    }
    let c = |n: &str| ExtTy::Int(format!("std::ffi::{n}"));
    Ok(match joined.as_str() {
        "int" | "signed" | "signed int" => c("c_int"),
        "unsigned" | "unsigned int" => c("c_uint"),
        "long" | "long int" | "signed long" => c("c_long"),
        "unsigned long" | "unsigned long int" => c("c_ulong"),
        "long long" | "long long int" => c("c_longlong"),
        "unsigned long long" => c("c_ulonglong"),
        "short" | "short int" => c("c_short"),
        "char" | "signed char" => c("c_char"),
        "unsigned char" => ExtTy::Int("u8".into()),
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
fn c_sig(head: &str, structs: &[String], module: &str) -> Result<Option<ExternFn>, (String, String)> {
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
    let ret = c_type_s(&ret_toks, false, structs, module).map_err(|e| (name.clone(), e))?;
    let inner = head[open + 1..close].trim();
    let mut params: Vec<ExtTy> = Vec::new();
    if !(inner.is_empty() || inner == "void") {
        let parts: Vec<&str> = split_top(inner);
        let mut i = 0;
        while i < parts.len() {
            let p = parts[i];
            if p.contains("...") {
                return Err((name, "variadic C functions are not supported".into()));
            }
            // `R (*cb)(A, B)`: a function pointer parameter.
            if let Some(star) = p.find("(*") {
                let ret_toks = tokens(&p[..star]);
                let Some(args_open) = p[star..].find(")(").map(|k| star + k + 1) else {
                    return Err((name, "unrecognized function pointer parameter".into()));
                };
                let args_close = p.rfind(')').unwrap_or(p.len());
                let r = c_type_s(&ret_toks, false, structs, module).map_err(|e| (name.clone(), e))?;
                let mut ps = Vec::new();
                let a = p[args_open + 1..args_close].trim();
                if !(a.is_empty() || a == "void") {
                    for q in a.split(',') {
                        let mut qt = tokens(q);
                        if qt.len() > 1 && !C_TYPE_WORDS.contains(&qt.last().unwrap().as_str()) {
                            qt.pop();
                        }
                        ps.push(c_type_s(&qt, false, structs, module).map_err(|e| (name.clone(), e))?);
                    }
                }
                params.push(ExtTy::CFn(ps, Box::new(r)));
                i += 1;
                continue;
            }
            let mut toks = tokens(p);
            // Drop the parameter name if present.
            if toks.len() > 1 && !C_TYPE_WORDS.contains(&toks.last().unwrap().as_str()) && !structs.contains(toks.last().unwrap()) {
                toks.pop();
            }
            // `T *data` followed by an integer length: one list argument.
            let is_ptr = toks.last().is_some_and(|t| t == "*") && toks.iter().filter(|t| *t == "*").count() == 1;
            if is_ptr && !toks.iter().any(|t| t == "char") {
                let elem_toks: Vec<String> = toks[..toks.len() - 1].to_vec();
                let elem = c_type_s(&elem_toks, false, structs, module).map_err(|e| (name.clone(), e))?;
                let len_ty = parts.get(i + 1).and_then(|n| {
                    let mut nt = tokens(n);
                    if nt.len() > 1 {
                        nt.pop();
                    }
                    match c_type(&nt, true) {
                        Ok(ExtTy::Int(r)) => Some(r),
                        _ => None,
                    }
                });
                let Some(len_ty) = len_ty else {
                    return Err((name, "a pointer parameter must be followed by its length (`T *data, size_t n`)".into()));
                };
                let mutable = !toks.iter().any(|t| t == "const");
                params.push(ExtTy::CArray(Box::new(elem), mutable, len_ty));
                i += 2;
                continue;
            }
            params.push(c_type_s(&toks, true, structs, module).map_err(|e| (name.clone(), e))?);
            i += 1;
        }
    }
    Ok(Some(ExternFn { name, params, ret, fallible: false, owner: None, recv: None }))
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
        let sigs: Vec<_> = heads.iter().map(|h| c_sig(h, &[], "")).collect();
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
            .filter_map(|i| if let syn::Item::Fn(f) = i { Some(rust_sig(&f.sig, &[], "m", None)) } else { None })
            .collect();
        assert!(sigs[0].is_ok() && sigs[1].is_ok());
        assert!(sigs[2].as_ref().unwrap().fallible);
        assert!(sigs[3].is_err());
    }
}

#[cfg(test)]
mod struct_tests {
    use super::*;

    #[test]
    fn c_typedef_structs() {
        let src = "typedef struct {\n    double x, y;\n} Vec2;\n\ntypedef struct {\n    unsigned char r;\n    unsigned char g;\n} Color;\n";
        let st = c_structs(src, "m");
        let names: Vec<&str> = st.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Vec2", "Color"], "{st:?}");
        assert_eq!(st[0].fields.len(), 2);
    }
}
