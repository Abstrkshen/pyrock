//! Code generation at the Rust/C boundary: module declarations, struct
//! handles, and value conversions in both directions.

use crate::{as_float, as_int, borrow, FnEmitter};
use pyfast_infer::native;
use pyfast_ir::*;
use std::fmt::Write;

/// Rust path of an extern module's items (`rs_name` / `c_name`).
fn module_path(m: &ExternModule) -> String {
    match m.lang {
        Lang::Rust => format!("rs_{}", m.name),
        Lang::C => format!("c_{}", m.name),
    }
}

/// The raw Rust type of a struct inside its module.
fn raw_struct(m: &ExternModule, idx: usize) -> String {
    format!("{}::{}", module_path(m), m.structs[idx].name)
}

/// A C declaration's parameter type (structs by their raw name).
fn c_decl_ty(m: &ExternModule, t: &ExtTy) -> String {
    match t {
        ExtTy::Struct { idx, .. } => m.structs[*idx].name.clone(),
        t => t.rust(),
    }
}

/// `mod` items for the linked Rust and C modules, and handle newtypes.
pub(crate) fn emit_decls(m: &Module, used: &std::collections::BTreeSet<usize>, out: &mut String) {
    for (i, x) in m.externs.iter().enumerate() {
        if !used.contains(&i) {
            continue;
        }
        out.push('\n');
        match x.lang {
            Lang::Rust => {
                writeln!(out, "/// `import {}` -> {}", x.name, x.path).unwrap();
                writeln!(out, "#[allow(dead_code)]\n#[path = {:?}]\nmod rs_{};", x.path, x.name).unwrap();
            }
            Lang::C => {
                writeln!(out, "/// `import {}` -> {}{}", x.name, x.path, x.header.as_ref().map(|h| format!(" (signatures from {h})")).unwrap_or_default()).unwrap();
                writeln!(out, "mod c_{} {{", x.name).unwrap();
                for st in &x.structs {
                    writeln!(out, "    #[repr(C)]\n    #[derive(Clone, Copy, Debug, Default)]\n    pub struct {} {{", st.name).unwrap();
                    for (f, ft) in &st.fields {
                        writeln!(out, "        pub {f}: {},", c_decl_ty(x, ft)).unwrap();
                    }
                    writeln!(out, "    }}").unwrap();
                }
                writeln!(out, "    extern \"C\" {{").unwrap();
                for f in x.funcs.iter().filter(|f| !f.name.ends_with("::__new__")) {
                    let mut ps = Vec::new();
                    for (i, p) in f.params.iter().enumerate() {
                        match p {
                            ExtTy::CArray(_, _, len) => {
                                ps.push(format!("a{i}: {}", p.rust()));
                                ps.push(format!("a{i}_len: {len}"));
                            }
                            p => ps.push(format!("a{i}: {}", c_decl_ty(x, p))),
                        }
                    }
                    let ret = if f.ret == ExtTy::Unit { String::new() } else { format!(" -> {}", c_decl_ty(x, &f.ret)) };
                    let _ = ret.len();
                    writeln!(out, "        pub fn {}({}){ret};", f.name, ps.join(", ")).unwrap();
                }
                writeln!(out, "    }}\n}}").unwrap();
            }
        }
        // Handles: shared, mutable references to a struct value.
        for (si, st) in x.structs.iter().enumerate() {
            let w = format!("E_{}_{}", x.name, st.name);
            let raw = raw_struct(x, si);
            writeln!(out, "\n/// Handle to `{}.{}`.\n#[derive(Clone)]\npub struct {w}(pub std::rc::Rc<std::cell::RefCell<{raw}>>);", x.name, st.name).unwrap();
            let fmt = if st.debug || x.lang == Lang::C {
                "rt::Str::from(format!(\"{:?}\", self.0.borrow()))".to_string()
            } else {
                format!("rt::Str::from(\"<{}.{} object>\")", x.name, st.name)
            };
            writeln!(out, "impl rt::conv::PyFmt for {w} {{\n    fn py_repr(&self) -> rt::Str {{\n        {fmt}\n    }}\n}}").unwrap();
            writeln!(out, "impl rt::coll::PyEq for {w} {{\n    fn py_eq(&self, o: &Self) -> bool {{\n        std::rc::Rc::ptr_eq(&self.0, &o.0)\n    }}\n}}").unwrap();
            writeln!(out, "impl rt::Truthy for {w} {{\n    fn truthy(&self) -> bool {{\n        true\n    }}\n}}").unwrap();
            writeln!(out, "impl {w} {{\n    pub fn is(&self, o: &Self) -> bool {{\n        std::rc::Rc::ptr_eq(&self.0, &o.0)\n    }}\n}}").unwrap();
            if x.lang == Lang::C {
                writeln!(out, "impl Default for {w} {{\n    fn default() -> Self {{\n        {w}(std::rc::Rc::new(std::cell::RefCell::new(Default::default())))\n    }}\n}}").unwrap();
            }
        }
    }
}

/// `build.rs` lines for C modules: compile, link libraries, pkg-config.
pub(crate) fn build_lines(x: &ExternModule, b: &mut String) {
    let dir = std::path::Path::new(&x.path).parent().map(|p| p.display().to_string()).unwrap_or_default();
    writeln!(b, "    println!(\"cargo:rerun-if-changed={}\");", x.path).unwrap();
    if let Some(h) = &x.header {
        writeln!(b, "    println!(\"cargo:rerun-if-changed={h}\");").unwrap();
    }
    writeln!(b, "    let mut build = cc::Build::new();\n    build.file({:?}).include({dir:?}).warnings(false);", x.path).unwrap();
    for pkg in &x.pkg_config {
        writeln!(
            b,
            "    if let Ok(o) = std::process::Command::new(\"pkg-config\").args([\"--cflags\", \"--libs\", {pkg:?}]).output() {{\n        for t in String::from_utf8_lossy(&o.stdout).split_whitespace() {{\n            if let Some(p) = t.strip_prefix(\"-I\") {{ build.include(p); }}\n            else if let Some(p) = t.strip_prefix(\"-L\") {{ println!(\"cargo:rustc-link-search=native={{p}}\"); }}\n            else if let Some(l) = t.strip_prefix(\"-l\") {{ println!(\"cargo:rustc-link-lib={{l}}\"); }}\n        }}\n    }}"
        )
        .unwrap();
    }
    writeln!(b, "    build.compile(\"pyfast_c_{}\");", x.name).unwrap();
    for lib in &x.link_libs {
        if let Some(dir) = lib.strip_prefix("search=") {
            writeln!(b, "    println!(\"cargo:rustc-link-search=native={dir}\");").unwrap();
        } else {
            writeln!(b, "    println!(\"cargo:rustc-link-lib={lib}\");").unwrap();
        }
    }
}

/// A Rust value of boundary type `r` as a native value of type `r.ty()`.
pub(crate) fn ext_out(code: &str, r: &ExtTy, n: &mut usize) -> String {
    *n += 1;
    let k = *n;
    match r {
        ExtTy::Int(rty) if rty == "i64" => code.to_string(),
        ExtTy::Int(_) => format!("rt::ext::int_from({code})?"),
        ExtTy::F32 => format!("({code} as f64)"),
        ExtTy::F64 | ExtTy::Bool | ExtTy::Unit => code.to_string(),
        ExtTy::OwnedString => format!("rt::Str::from({code})"),
        ExtTy::StrRef => format!("rt::Str::from({code})"),
        ExtTy::CStr => format!("unsafe {{ rt::ext::from_cstr({code})? }}"),
        ExtTy::Vec(e) | ExtTy::Slice(e) | ExtTy::SliceMut(e, _) | ExtTy::CArray(e, _, _) => {
            format!("{{ let mut __o{k} = Vec::new(); for __y{k} in {code} {{ __o{k}.push({}); }} rt::List::<{}>::from_vec(__o{k}) }}", ext_out(&format!("__y{k}"), e, n), e.ty().rust())
        }
        ExtTy::Bytes(Pass::Value) => format!("rt::py::bytes(&{code})"),
        ExtTy::Bytes(_) => format!("rt::List::<i64>::from_vec({code}.iter().map(|b| *b as i64).collect())"),
        ExtTy::Opt(e) => format!("match {code} {{ Some(__y{k}) => Some({}), None => None }}", ext_out(&format!("__y{k}"), e, n)),
        ExtTy::Tuple(es) => {
            let parts: Vec<String> = es.iter().enumerate().map(|(i, e)| ext_out(&format!("__t{k}.{i}"), e, n)).collect();
            format!("{{ let __t{k} = {code}; ({},) }}", parts.join(", "))
        }
        ExtTy::Map(kt, vt, _) => {
            let (ko, vo) = (ext_out(&format!("__k{k}"), kt, n), ext_out(&format!("__v{k}"), vt, n));
            format!("{{ let mut __o{k} = Vec::new(); for (__k{k}, __v{k}) in {code} {{ __o{k}.push(({ko}, {vo})); }} rt::Dict::from_pairs(__o{k}) }}")
        }
        ExtTy::Struct { name, .. } => format!("{name}(std::rc::Rc::new(std::cell::RefCell::new({code})))"),
        ExtTy::Callback(..) | ExtTy::CFn(..) => unreachable!("callbacks are parameters only"),
    }
}

impl FnEmitter<'_, '_> {
    /// A native value `x` of type `t` as a Rust value of boundary type `e`
    /// (an expression; may use `?`).
    pub(crate) fn conv_in(&mut self, x: &str, t: Ty, e: &ExtTy) -> String {
        if t.is_never() {
            return format!("{{ let __n: rt::Never = {x}; match __n {{}} }}");
        }
        let (x, t) = if matches!(e, ExtTy::Opt(_)) { (x.to_string(), t) } else { crate::unopt(x.to_string(), t, &format!("expected {}, got None", e.ty())) };
        let k = self.fresh();
        match e {
            ExtTy::Int(rty) => {
                let v = if t == Ty::Obj { format!("rt::py::from_py::<i64>(&{})?", borrow(&x)) } else { as_int(&x, t) };
                if rty == "i64" { v } else { format!("rt::ext::int_to::<{rty}>({v}, {rty:?})?") }
            }
            ExtTy::F64 => if t == Ty::Obj { format!("rt::py::to_f64(&{})?", borrow(&x)) } else { as_float(&x, t) },
            ExtTy::F32 => format!("({} as f32)", if t == Ty::Obj { format!("rt::py::to_f64(&{})?", borrow(&x)) } else { as_float(&x, t) }),
            ExtTy::Bool if t == Ty::Obj => format!("rt::py::to_bool(&{})?", borrow(&x)),
            ExtTy::Bool => x,
            ExtTy::StrRef if t == Ty::Obj => format!("&*rt::py::to_str(&{})?", borrow(&x)),
            ExtTy::StrRef => format!("&*{}", borrow(&x)),
            ExtTy::OwnedString if t == Ty::Obj => format!("rt::py::to_str(&{})?.to_string()", borrow(&x)),
            ExtTy::OwnedString => format!("{}.to_string()", borrow(&x)),
            ExtTy::CStr if t == Ty::Obj => format!("rt::ext::cstring(&rt::py::to_str(&{})?)?.as_ptr()", borrow(&x)),
            ExtTy::CStr => format!("rt::ext::cstring(&{})?.as_ptr()", borrow(&x)),
            ExtTy::Unit => "()".into(),
            ExtTy::Vec(el) | ExtTy::Slice(el) | ExtTy::SliceMut(el, _) | ExtTy::CArray(el, _, _) => {
                let xt = native::iter_elem(t).unwrap_or(Ty::Obj);
                let inner = self.conv_in(&format!("__x{k}"), xt, el);
                let v = format!("{{ let mut __v{k}: Vec<{}> = Vec::new(); for __x{k} in {} {{ __v{k}.push({inner}); }} __v{k} }}", el.rust(), crate::coll::iter_vec(&x, t));
                if matches!(e, ExtTy::Slice(_)) { format!("&{v}") } else { v }
            }
            ExtTy::Bytes(pass) => {
                let xt = native::iter_elem(t).unwrap_or(Ty::Obj);
                let inner = self.conv_in(&format!("__x{k}"), xt, &ExtTy::Int("u8".into()));
                let v = format!("{{ let mut __v{k}: Vec<u8> = Vec::new(); for __x{k} in {} {{ __v{k}.push({inner}); }} __v{k} }}", crate::coll::iter_vec(&x, t));
                if *pass == Pass::Value { v } else { format!("&{v}[..]") }
            }
            ExtTy::Opt(el) => match t {
                Ty::None => "None".into(),
                _ if t.as_opt().is_some() => {
                    let inner = self.conv_in(&format!("__x{k}"), t.as_opt().unwrap(), el);
                    format!("match {x} {{ Some(__x{k}) => Some({inner}), None => None }}")
                }
                _ => format!("Some({})", self.conv_in(&x, t, el)),
            },
            ExtTy::Tuple(es) => {
                let ts = t.tuple_items().unwrap();
                let parts: Vec<String> = es.iter().enumerate().map(|(i, el)| self.conv_in(&format!("__t{k}.{i}"), ts[i], el)).collect();
                format!("{{ let __t{k} = {x}; ({},) }}", parts.join(", "))
            }
            ExtTy::Map(kt, vt, _) => {
                let (tk, tv) = t.dict_kv().unwrap();
                let (kc, vc) = (self.conv_in(&format!("__k{k}"), tk, kt), self.conv_in(&format!("__v{k}"), tv, vt));
                format!("{{ let mut __m{k} = {}::new(); for (__k{k}, __v{k}) in {}.items_vec() {{ __m{k}.insert({kc}, {vc}); }} __m{k} }}", e.rust().split('<').next().unwrap(), borrow(&x))
            }
            ExtTy::Struct { pass, .. } => match pass {
                Pass::Ref => format!("&*{}.0.borrow()", borrow(&x)),
                Pass::Mut => format!("&mut *{}.0.borrow_mut()", borrow(&x)),
                Pass::Value => format!("{}.0.borrow().clone()", borrow(&x)),
            },
            ExtTy::Callback(..) | ExtTy::CFn(..) => unreachable!("callbacks are handled by extern_call"),
        }
    }

    /// A call into Rust/C: `func` of module `ext`, with the receiver of a
    /// method already evaluated (`recv`).
    pub(crate) fn extern_call(&mut self, ext: usize, func: usize, recv: Option<(String, Ty)>, args: &[Expr]) -> String {
        let m = self.prog.module.externs[ext].clone();
        let f = m.funcs[func].clone();
        let path = module_path(&m);
        let mut pre = String::new();
        let mut post = String::new();
        let mut conv: Vec<String> = Vec::new();
        if let (Some((rc, _)), Some(p)) = (&recv, f.recv) {
            write!(pre, "let __self = {rc}; ").unwrap();
            conv.push(match p {
                Pass::Ref => "&*__self.0.borrow()".into(),
                Pass::Mut => "&mut *__self.0.borrow_mut()".into(),
                Pass::Value => "__self.0.borrow().clone()".into(),
            });
        }
        // C struct constructor: a struct literal.
        if m.lang == Lang::C && f.name.ends_with("::__new__") {
            let si = f.owner.unwrap();
            let st = &m.structs[si];
            let mut fields = Vec::new();
            for ((fname, ft), a) in st.fields.iter().zip(args) {
                let (c, t) = self.expr(a);
                fields.push(format!("{fname}: {}", self.conv_in(&c, t, ft)));
            }
            return format!("E_{}_{}(std::rc::Rc::new(std::cell::RefCell::new({} {{ {} }})))", m.name, st.name, raw_struct(&m, si), fields.join(", "));
        }
        let mut uses_callbacks = false;
        for (i, (p, a)) in f.params.iter().zip(args).enumerate() {
            let (c, t) = self.expr(a);
            match p {
                ExtTy::SliceMut(..) | ExtTy::CArray(_, true, _) => {
                    let is_vec = matches!(p, ExtTy::SliceMut(_, true));
                    let el = match p {
                        ExtTy::SliceMut(el, _) | ExtTy::CArray(el, _, _) => el.clone(),
                        _ => unreachable!(),
                    };
                    write!(pre, "let __l{i} = {c}; let mut __m{i} = {}; ", self.conv_in(&format!("__l{i}.clone()"), t, p)).unwrap();
                    match p {
                        ExtTy::SliceMut(..) if is_vec => conv.push(format!("&mut __m{i}")),
                        ExtTy::SliceMut(..) => conv.push(format!("&mut __m{i}[..]")),
                        ExtTy::CArray(_, _, len) => conv.push(format!("__m{i}.as_mut_ptr(), __m{i}.len() as {len}")),
                        _ => unreachable!(),
                    }
                    let mut n = 0;
                    let back = ext_out(&format!("__y{i}"), &el, &mut n);
                    if t == Ty::Obj {
                        // A Python sequence (list, numpy array): `obj[:] = values`.
                        let boxed = crate::boxed(&back, el.ty());
                        write!(post, "{{ let mut __b{i} = Vec::new(); for __y{i} in __m{i} {{ __b{i}.push({boxed}); }} rt::py::set_all(&__l{i}, __b{i})?; }} ").unwrap();
                    } else {
                        write!(post, "{{ let mut __b{i} = Vec::new(); for __y{i} in __m{i} {{ __b{i}.push({back}); }} __l{i}.clear(); __l{i}.extend(__b{i}); }} ").unwrap();
                    }
                }
                ExtTy::CArray(el, false, len) => {
                    write!(pre, "let __m{i}: Vec<{}> = {}; ", el.rust(), self.conv_in(&c, t, &ExtTy::Vec(el.clone()))).unwrap();
                    conv.push(format!("__m{i}.as_ptr(), __m{i}.len() as {len}"));
                }
                ExtTy::CFn(ps, r) => {
                    uses_callbacks = true;
                    let (fid, _) = t.as_closure().expect("callback is a native function value");
                    let env = pyfast_infer::spec::env_offset(&self.prog.module.funcs[fid]) == 1;
                    let mut key: Vec<Ty> = if env { vec![t] } else { vec![] };
                    key.extend(ps.iter().map(|x| x.ty()));
                    let spec = self.prog.spec(&pyfast_infer::SpecKey { func: fid, args: key }).expect("callback specialization").clone();
                    let rt_ty = match spec.ret {
                        pyfast_infer::Slot::Known(x) => x,
                        pyfast_infer::Slot::Boxed(..) => Ty::Obj,
                        _ => Ty::None,
                    };
                    let mut n = 0;
                    let k = self.fresh();
                    let params: Vec<String> = ps.iter().enumerate().map(|(j, x)| format!("__a{j}: {}", x.rust())).collect();
                    let names: Vec<String> = (0..ps.len()).map(|j| format!("__a{j}")).collect();
                    let tys: Vec<String> = ps.iter().map(|x| x.rust()).collect();
                    let mut call_args: Vec<String> = if env { vec![format!("__f{i}.clone()")] } else { vec![] };
                    call_args.extend(ps.iter().enumerate().map(|(j, x)| ext_out(&format!("__a{j}"), x, &mut n)));
                    let ret_conv = if **r == ExtTy::Unit { "()".to_string() } else { self.conv_in("__v", rt_ty, r) };
                    let rr = r.rust();
                    let dynty = format!("dyn Fn({}) -> {rr}", tys.join(", "));
                    // A thread-local stack per call site keeps the closure for the
                    // `extern "C"` trampoline (re-entrant calls push again).
                    self.cfn_items.push(format!(
                        "thread_local! {{ static CB{k}: std::cell::RefCell<Vec<std::rc::Rc<{dynty}>>> = const {{ std::cell::RefCell::new(Vec::new()) }}; }}\nextern \"C\" fn tramp{k}({}) -> {rr} {{ let f = CB{k}.with(|s| s.borrow().last().cloned().expect(\"callback\")); f({}) }}",
                        params.join(", "),
                        names.join(", ")
                    ));
                    write!(
                        pre,
                        "let __f{i} = {c}; let __cb{i} = move |{}| -> {rr} {{ let __res: PyResult<{rr}> = (|| -> PyResult<{rr}> {{ let __v = {}({})?; Ok({ret_conv}) }})(); match __res {{ Ok(__v) => __v, Err(__e) => {{ rt::ext::stash(__e); Default::default() }} }} }}; let __bx{i}: Box<{dynty} + '_> = Box::new(__cb{i}); let __st{i}: Box<{dynty} + 'static> = unsafe {{ std::mem::transmute(__bx{i}) }}; CB{k}.with(|s| s.borrow_mut().push(std::rc::Rc::from(__st{i}))); ",
                        params.join(", "),
                        spec.mangled,
                        call_args.join(", ")
                    )
                    .unwrap();
                    conv.push(format!("tramp{k}"));
                    write!(post, "CB{k}.with(|s| s.borrow_mut().pop()); ").unwrap();
                }
                ExtTy::Callback(ps, r, dyn_ref) => {
                    uses_callbacks = true;
                    let (fid, _) = t.as_closure().expect("callback is a native function value");
                    let env = pyfast_infer::spec::env_offset(&self.prog.module.funcs[fid]) == 1;
                    let mut key: Vec<Ty> = if env { vec![t] } else { vec![] };
                    key.extend(ps.iter().map(|x| x.ty()));
                    let spec = self.prog.spec(&pyfast_infer::SpecKey { func: fid, args: key }).expect("callback specialization").clone();
                    let rt_ty = match spec.ret {
                        pyfast_infer::Slot::Known(x) => x,
                        pyfast_infer::Slot::Boxed(..) => Ty::Obj,
                        _ => Ty::None,
                    };
                    let mut n = 0;
                    let params: Vec<String> = ps.iter().enumerate().map(|(j, x)| format!("__a{j}: {}", x.rust())).collect();
                    let mut call_args: Vec<String> = if env { vec![format!("__f{i}.clone()")] } else { vec![] };
                    call_args.extend(ps.iter().enumerate().map(|(j, x)| ext_out(&format!("__a{j}"), x, &mut n)));
                    let ret_conv = if **r == ExtTy::Unit { "()".to_string() } else { self.conv_in("__v", rt_ty, r) };
                    write!(
                        pre,
                        "let __f{i} = {c}; let __cb{i} = |{}| -> {} {{ let __res: PyResult<{}> = (|| -> PyResult<{}> {{ let __v = {}({})?; Ok({ret_conv}) }})(); match __res {{ Ok(__v) => __v, Err(__e) => {{ rt::ext::stash(__e); Default::default() }} }} }}; ",
                        params.join(", "),
                        r.rust(),
                        r.rust(),
                        r.rust(),
                        spec.mangled,
                        call_args.join(", ")
                    )
                    .unwrap();
                    conv.push(if *dyn_ref { format!("&__cb{i}") } else { format!("__cb{i}") });
                }
                p => {
                    let code = self.conv_in(&c, t, p);
                    conv.push(code);
                }
            }
        }
        let call = match m.lang {
            Lang::Rust => format!("{path}::{}({})", f.name, conv.join(", ")),
            Lang::C => format!("unsafe {{ {path}::{}({}) }}", f.name, conv.join(", ")),
        };
        let call = if f.fallible { format!("{call}.map_err(rt::ext::rust_err)?") } else { call };
        if uses_callbacks {
            write!(post, "rt::ext::check()?; ").unwrap();
        }
        let mut n = 0;
        let out = ext_out("__r", &f.ret, &mut n);
        if pre.is_empty() && post.is_empty() {
            // Simple calls keep the old shape (readable output).
            return ext_out(&call, &f.ret, &mut 0);
        }
        format!("{{ {pre}let __r = {call}; {post}{out} }}")
    }

    /// `handle.field`
    pub(crate) fn ext_field(&mut self, obj: &Expr, name: &str) -> String {
        let (o, ot) = self.expr(obj);
        let (ext, si) = ot.as_ext_struct().unwrap();
        let ft = self.prog.module.externs[ext].structs[si].fields.iter().find(|(n, _)| n == name).unwrap().1.clone();
        ext_out(&format!("{}.0.borrow().{name}.clone()", borrow(&o)), &ft, &mut 0)
    }

    /// `handle.field = value`
    pub(crate) fn ext_set_field(&mut self, obj: &Expr, name: &str, value: &Expr, ind: &str) -> String {
        let ot = self.ty(obj);
        let (ext, si) = ot.as_ext_struct().unwrap();
        let ft = self.prog.module.externs[ext].structs[si].fields.iter().find(|(n, _)| n == name).unwrap().1.clone();
        let (v, vt) = self.expr(value);
        let cv = self.conv_in("__v", vt, &ft);
        let o = self.expr(obj).0;
        format!("{ind}{{\n{ind}    let __v = {v};\n{ind}    let __c = {cv};\n{ind}    {}.0.borrow_mut().{name} = __c;\n{ind}}}", borrow(&o))
    }
}
