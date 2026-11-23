//! Record dicts: a dict literal with fixed string keys whose keys keep their
//! own types becomes a generated struct, shared like a dict
//! (`Rc<RefCell<..>>`).

use crate::{boxed, coll::convert};
use pyrock_infer::{native, Program};
use pyrock_ir::*;
use std::fmt::Write;

/// Emit a struct for every record type the generated code mentions.
pub(crate) fn emit_records(prog: &Program, uses_python: bool, out: &mut String) {
    let mut done: Vec<Ty> = Vec::new();
    loop {
        let todo: Vec<Ty> = Ty::all_records().into_iter().filter(|r| !done.contains(r) && out.contains(r.rust())).collect();
        if todo.is_empty() {
            break;
        }
        for r in todo {
            done.push(r);
            let mut s = String::new();
            emit_one(prog, r, uses_python, &mut s);
            out.push_str(&s);
        }
    }
}

fn emit_one(prog: &Program, r: Ty, uses_python: bool, out: &mut String) {
    let fs = r.as_record().unwrap();
    let n = r.rust();
    let any_opt = fs.iter().any(|f| f.optional);
    writeln!(out, "\n/// `{}`: a dict with fixed keys, as a struct.", r.py_name()).unwrap();
    writeln!(out, "#[derive(Clone, Default)]\nstruct {n}(std::rc::Rc<std::cell::RefCell<{n}_d>>);").unwrap();
    let mut fields: Vec<String> = fs.iter().enumerate().map(|(i, f)| if f.optional { format!("f{i}: Option<{}>", f.ty.rust()) } else { format!("f{i}: {}", f.ty.rust()) }).collect();
    if any_opt {
        fields.push("order: Vec<u8>".into());
    }
    writeln!(out, "#[derive(Clone, Default)]\nstruct {n}_d {{ {} }}", fields.join(", ")).unwrap();
    let req: Vec<usize> = (0..fs.len()).filter(|i| !fs[*i].optional).collect();
    let params: Vec<String> = req.iter().map(|i| format!("f{i}: {}", fs[*i].ty.rust())).collect();
    let mut inits: Vec<String> = (0..fs.len()).map(|i| if fs[i].optional { format!("f{i}: None") } else { format!("f{i}") }).collect();
    if any_opt {
        inits.push("order: Vec::new()".into());
    }
    // `each(|key, value_code| item)` over present keys in insertion order: the
    // literal's keys, then added keys in the order they were added.
    let each = |item: &dyn Fn(&str, &str, Ty) -> String| -> String {
        let mut code = String::new();
        for i in &req {
            writeln!(code, "        __v.push({});", item(fs[*i].key, &format!("d.f{i}.clone()"), fs[*i].ty)).unwrap();
        }
        if any_opt {
            let arms: Vec<String> = (0..fs.len())
                .filter(|i| fs[*i].optional)
                .map(|i| format!("{i} => __v.push({}),", item(fs[i].key, &format!("d.f{i}.clone().unwrap()"), fs[i].ty)))
                .collect();
            writeln!(code, "        for &i in &d.order {{\n            match i {{ {} _ => unreachable!() }}\n        }}", arms.join(" ")).unwrap();
        }
        code
    };
    writeln!(out, "impl {n} {{").unwrap();
    writeln!(out, "    fn new({}) -> Self {{\n        {n}(std::rc::Rc::new(std::cell::RefCell::new({n}_d {{ {} }})))\n    }}", params.join(", "), inits.join(", ")).unwrap();
    writeln!(out, "    fn is(&self, o: &Self) -> bool {{\n        std::rc::Rc::ptr_eq(&self.0, &o.0)\n    }}").unwrap();
    let len = if any_opt { format!("{} + self.0.borrow().order.len() as i64", req.len()) } else { req.len().to_string() };
    writeln!(out, "    fn len(&self) -> i64 {{\n        {len}\n    }}").unwrap();
    let key_arms: Vec<String> = (0..fs.len()).map(|i| if fs[i].optional { format!("{:?} => self.0.borrow().f{i}.is_some(),", fs[i].key) } else { format!("{:?} => true,", fs[i].key) }).collect();
    writeln!(out, "    fn has_key(&self, k: &str) -> bool {{\n        match k {{ {} _ => false }}\n    }}", key_arms.join(" ")).unwrap();
    writeln!(out, "    /// The keys (iterating a dict).\n    fn to_vec(&self) -> Vec<rt::Str> {{\n        let d = self.0.borrow();\n        let mut __v = Vec::new();\n{}        let _ = &d;\n        __v\n    }}", each(&|k, _, _| format!("rt::lit!({k:?})"))).unwrap();
    for i in (0..fs.len()).filter(|i| fs[*i].optional) {
        let t = fs[i].ty.rust();
        let missing = format!("rt::Exc::new(\"KeyError\", {:?})", py_str_repr(fs[i].key));
        writeln!(out, "    fn get_f{i}(&self) -> PyResult<{t}> {{\n        self.0.borrow().f{i}.clone().ok_or_else(|| {missing})\n    }}").unwrap();
        writeln!(out, "    fn opt_f{i}(&self) -> Option<{t}> {{\n        self.0.borrow().f{i}.clone()\n    }}").unwrap();
        writeln!(out, "    fn set_f{i}(&self, v: {t}) {{\n        let mut d = self.0.borrow_mut();\n        if d.f{i}.is_none() {{\n            d.order.push({i});\n        }}\n        d.f{i} = Some(v);\n    }}").unwrap();
        writeln!(out, "    fn del_f{i}(&self) -> PyResult<()> {{\n        let mut d = self.0.borrow_mut();\n        if d.f{i}.take().is_none() {{\n            return Err({missing});\n        }}\n        d.order.retain(|x| *x != {i});\n        Ok(())\n    }}").unwrap();
    }
    if let Some(dt) = native::record_dict(r, prog.boxing()).filter(|d| uses_python || !d.mentions_obj()) {
        let (_, v) = dt.dict_kv().unwrap();
        let body = each(&|k, code, t| format!("(rt::lit!({k:?}), {})", convert(code, t, v)));
        writeln!(out, "    /// A copy as a plain dict.\n    fn to_dict(&self) -> PyResult<{}> {{\n        let d = self.0.borrow();\n        let mut __v = Vec::new();\n{body}        Ok(rt::Dict::from_pairs(__v))\n    }}", dt.rust()).unwrap();
    }
    writeln!(out, "}}").unwrap();
    let reprs = each(&|k, code, _| format!("format!(\"{{}}: {{}}\", {:?}, rt::conv::repr_of(&{code}))", py_str_repr(k)));
    writeln!(out, "impl rt::conv::PyFmt for {n} {{\n    fn py_repr(&self) -> rt::Str {{\n        let d = self.0.borrow();\n        let mut __v: Vec<String> = Vec::new();\n{reprs}        rt::Str::from(format!(\"{{{{{{}}}}}}\", __v.join(\", \")))\n    }}\n}}").unwrap();
    // Dict equality ignores insertion order.
    let eqs: Vec<String> = (0..fs.len()).map(|i| if fs[i].optional {
        format!("(match (&a.f{i}, &b.f{i}) {{ (Some(x), Some(y)) => rt::coll::eq(x, y), (None, None) => true, _ => false }})")
    } else {
        format!("rt::coll::eq(&a.f{i}, &b.f{i})")
    }).collect();
    writeln!(out, "impl rt::coll::PyEq for {n} {{\n    fn py_eq(&self, o: &Self) -> bool {{\n        if self.is(o) {{\n            return true;\n        }}\n        let (a, b) = (self.0.borrow(), o.0.borrow());\n        {}\n    }}\n}}", eqs.join(" && ")).unwrap();
    writeln!(out, "impl rt::Truthy for {n} {{\n    fn truthy(&self) -> bool {{\n        self.len() > 0\n    }}\n}}").unwrap();
    if uses_python && native::record_dict(r, true).is_some() {
        let items = each(&|k, code, t| format!("(rt::pystr!({k:?}), {})", boxed(code, t)));
        writeln!(out, "impl rt::py::ToPy for {n} {{\n    fn to_py(&self) -> PyResult<rt::py::Obj> {{\n        let d = self.0.borrow();\n        let mut __v = Vec::new();\n{items}        rt::py::dict(__v)\n    }}\n}}").unwrap();
    }
}

/// Python's `repr` of a str (single quotes unless the text has one).
pub(crate) fn py_str_repr(s: &str) -> String {
    let q = if s.contains('\'') && !s.contains('"') { '"' } else { '\'' };
    let mut out = String::new();
    out.push(q);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if c == q => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out.push(q);
    out
}
