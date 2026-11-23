//! Rewrites of the CPython AST (as JSON) applied before lowering: constructs
//! expressed with ones the lowerer already compiles, so every later pass
//! (scopes, inference, codegen) handles them for free.
//!
//! * `for`/`while ... else`: a flag set before each `break` of that loop;
//!   the `else` body runs when the flag is still false.
//! * `...` as a statement (a placeholder body): `pass`.
//! * `[*a, b]`, `(*a, b)`, `{*a, b}`, `{**d, "k": v}`: concatenations of
//!   `list(a)` / `set(a)` / `dict(d)` and plain literals, left to right
//!   (later dict keys win, as in CPython).

use serde_json::{json, Value};

pub fn rewrite(module: &mut Value) {
    let typing = typing_names(module);
    if !typing.is_empty() {
        strip_typing_decorators(module, &typing);
    }
    let mut n = 0;
    walk(module, &mut n);
}

/// Local names for `typing`'s `overload`, `override` and `final` (also from
/// `typing_extensions`), and for the modules themselves (as `typing.X`).
fn typing_names(module: &Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for st in module["body"].as_array().into_iter().flatten() {
        let from_typing = |m: &Value| matches!(m.as_str(), Some("typing" | "typing_extensions"));
        match ty(st) {
            "ImportFrom" if from_typing(&st["module"]) => {
                for a in st["names"].as_array().into_iter().flatten() {
                    let name = a["name"].as_str().unwrap_or("");
                    if matches!(name, "overload" | "override" | "final") {
                        out.push((a["asname"].as_str().unwrap_or(name).to_string(), name.to_string()));
                    }
                }
            }
            "Import" => {
                for a in st["names"].as_array().into_iter().flatten() {
                    if from_typing(&a["name"]) {
                        out.push((a["asname"].as_str().or(a["name"].as_str()).unwrap_or("").to_string(), String::new()));
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Which typing decorator `d` is (`overload`, `override`, `final`), if any.
fn typing_decorator(d: &Value, names: &[(String, String)]) -> Option<String> {
    match ty(d) {
        "Name" => names.iter().find(|(l, r)| !r.is_empty() && d["id"] == l.as_str()).map(|x| x.1.clone()),
        "Attribute" if ty(&d["value"]) == "Name" => {
            let attr = d["attr"].as_str().unwrap_or("");
            let is_mod = names.iter().any(|(l, r)| r.is_empty() && d["value"]["id"] == l.as_str());
            (is_mod && matches!(attr, "overload" | "override" | "final")).then(|| attr.to_string())
        }
        _ => None,
    }
}

/// `@override` / `@final` only inform type checkers: dropped. `@overload`
/// stubs are replaced by the real definition that follows: removed.
fn strip_typing_decorators(v: &mut Value, names: &[(String, String)]) {
    match v {
        Value::Array(items) => items.iter_mut().for_each(|x| strip_typing_decorators(x, names)),
        Value::Object(map) => {
            for key in ["body", "orelse", "finalbody"] {
                if let Some(Value::Array(stmts)) = map.get_mut(key) {
                    stmts.retain(|st| {
                        !(matches!(ty(st), "FunctionDef" | "AsyncFunctionDef")
                            && st["decorator_list"].as_array().is_some_and(|ds| ds.iter().any(|d| typing_decorator(d, names).as_deref() == Some("overload"))))
                    });
                }
            }
            if let Some(Value::Array(ds)) = map.get_mut("decorator_list") {
                ds.retain(|d| typing_decorator(d, names).is_none());
            }
            for (_, child) in map.iter_mut() {
                strip_typing_decorators(child, names);
            }
        }
        _ => {}
    }
}

fn walk(v: &mut Value, n: &mut usize) {
    match v {
        Value::Array(items) => {
            for it in items.iter_mut() {
                walk(it, n);
            }
        }
        Value::Object(map) => {
            for (_, child) in map.iter_mut() {
                walk(child, n);
            }
            for key in ["body", "orelse", "finalbody"] {
                if let Some(Value::Array(stmts)) = map.get_mut(key) {
                    expand_stmts(stmts, n);
                }
            }
        }
        _ => {}
    }
    if let Some(e) = unstar_literal(v) {
        *v = e;
    }
}

fn ty(v: &Value) -> &str {
    v["_t"].as_str().unwrap_or("")
}

fn name(id: &str, loc: &Value, store: bool) -> Value {
    json!({"_t": "Name", "id": id, "ctx": {"_t": if store { "Store" } else { "Load" }}, "_loc": loc})
}

fn assign_bool(flag: &str, b: bool, loc: &Value) -> Value {
    json!({"_t": "Assign", "targets": [name(flag, loc, true)], "value": {"_t": "Constant", "value": {"kind": "bool", "value": b}, "_loc": loc}, "_loc": loc})
}

/// Statement-level rewrites within one statement list.
fn expand_stmts(stmts: &mut Vec<Value>, n: &mut usize) {
    let mut out = Vec::with_capacity(stmts.len());
    for mut st in std::mem::take(stmts) {
        let t = ty(&st).to_string();
        if t == "Expr" && st["value"]["_t"] == "Constant" && st["value"]["value"]["type"] == "ellipsis" {
            out.push(json!({"_t": "Pass", "_loc": st["_loc"]}));
            continue;
        }
        let has_else = st["orelse"].as_array().is_some_and(|a| !a.is_empty());
        if matches!(t.as_str(), "For" | "AsyncFor" | "While") && has_else {
            *n += 1;
            let flag = format!("__loop_broke_{n}");
            let loc = st["_loc"].clone();
            let orelse = std::mem::replace(&mut st["orelse"], json!([]));
            if let Some(Value::Array(body)) = st.get_mut("body") {
                flag_breaks(body, &flag);
            }
            out.push(assign_bool(&flag, false, &loc));
            out.push(st);
            out.push(json!({
                "_t": "If",
                "test": {"_t": "UnaryOp", "op": {"_t": "Not"}, "operand": name(&flag, &loc, false), "_loc": loc},
                "body": orelse,
                "orelse": [],
                "_loc": loc,
            }));
            continue;
        }
        out.push(st);
    }
    *stmts = out;
}

/// `break` statements that leave the loop whose body is `stmts` (not those
/// of nested loops, functions or classes) first set `flag`.
fn flag_breaks(stmts: &mut Vec<Value>, flag: &str) {
    let mut out = Vec::with_capacity(stmts.len());
    for mut st in std::mem::take(stmts) {
        match ty(&st) {
            "Break" => {
                out.push(assign_bool(flag, true, &st["_loc"]));
                out.push(st);
                continue;
            }
            "FunctionDef" | "AsyncFunctionDef" | "ClassDef" => {}
            // A nested loop's own breaks are its own; its `else` belongs to us.
            "For" | "AsyncFor" | "While" => {
                if let Some(Value::Array(b)) = st.get_mut("orelse") {
                    flag_breaks(b, flag);
                }
            }
            _ => {
                for key in ["body", "orelse", "finalbody"] {
                    if let Some(Value::Array(b)) = st.get_mut(key) {
                        flag_breaks(b, flag);
                    }
                }
                if let Some(Value::Array(hs)) = st.get_mut("handlers") {
                    for h in hs {
                        if let Some(Value::Array(b)) = h.get_mut("body") {
                            flag_breaks(b, flag);
                        }
                    }
                }
                if let Some(Value::Array(cases)) = st.get_mut("cases") {
                    for c in cases {
                        if let Some(Value::Array(b)) = c.get_mut("body") {
                            flag_breaks(b, flag);
                        }
                    }
                }
            }
        }
        out.push(st);
    }
    *stmts = out;
}

fn call(f: &str, arg: Value, loc: &Value) -> Value {
    json!({"_t": "Call", "func": name(f, loc, false), "args": [arg], "keywords": [], "_loc": loc})
}

fn binop(op: &str, l: Value, r: Value, loc: &Value) -> Value {
    json!({"_t": "BinOp", "left": l, "op": {"_t": op}, "right": r, "_loc": loc})
}

/// `[*a, b]` and friends (in a load context) as concatenations.
fn unstar_literal(v: &Value) -> Option<Value> {
    let t = ty(v);
    let loc = &v["_loc"];
    match t {
        "List" | "Tuple" | "Set" => {
            if v["ctx"]["_t"] == "Store" || v["ctx"]["_t"] == "Del" {
                return None;
            }
            let elts = v["elts"].as_array()?;
            if !elts.iter().any(|e| ty(e) == "Starred") {
                return None;
            }
            let (conv, op, lit) = if t == "Set" { ("set", "BitOr", "Set") } else { ("list", "Add", "List") };
            let mut parts: Vec<Value> = Vec::new();
            let mut run: Vec<Value> = Vec::new();
            let flush = |run: &mut Vec<Value>, parts: &mut Vec<Value>| {
                if !run.is_empty() {
                    let mut lit_v = json!({"_t": lit, "elts": std::mem::take(run), "_loc": loc});
                    if lit == "List" {
                        lit_v["ctx"] = json!({"_t": "Load"});
                    }
                    parts.push(lit_v);
                }
            };
            for e in elts {
                if ty(e) == "Starred" {
                    flush(&mut run, &mut parts);
                    parts.push(call(conv, e["value"].clone(), loc));
                } else {
                    run.push(e.clone());
                }
            }
            flush(&mut run, &mut parts);
            let mut acc = parts.remove(0);
            for p in parts {
                acc = binop(op, acc, p, loc);
            }
            Some(if t == "Tuple" { call("tuple", acc, loc) } else { acc })
        }
        "Dict" => {
            let keys = v["keys"].as_array()?;
            if !keys.iter().any(|k| k.is_null()) {
                return None;
            }
            let values = v["values"].as_array()?;
            let mut parts: Vec<Value> = Vec::new();
            let (mut rk, mut rv): (Vec<Value>, Vec<Value>) = (vec![], vec![]);
            for (k, val) in keys.iter().zip(values) {
                if k.is_null() {
                    if !rk.is_empty() {
                        let lit = json!({"_t": "Dict", "keys": std::mem::take(&mut rk), "values": std::mem::take(&mut rv), "_loc": loc});
                        parts.push(call("dict", lit, loc));
                    }
                    parts.push(call("dict", val.clone(), loc));
                } else {
                    rk.push(k.clone());
                    rv.push(val.clone());
                }
            }
            // Literal parts as plain dicts too: a string-keyed literal would
            // otherwise be a record, which `|` doesn't combine with a dict.
            if !rk.is_empty() {
                parts.push(call("dict", json!({"_t": "Dict", "keys": rk, "values": rv, "_loc": loc}), loc));
            }
            let mut acc = parts.remove(0);
            for p in parts {
                acc = binop("BitOr", acc, p, loc);
            }
            Some(acc)
        }
        _ => None,
    }
}
