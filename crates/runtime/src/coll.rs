//! Native collections with Python semantics.
//!
//! * `List<T>`: a shared, mutable vector. Cloning a `List` clones the
//!   reference, so aliases observe each other's mutations exactly like
//!   Python names bound to the same list.
//! * `Dict<K, V>`: insertion-ordered hash map (CPython 3.7+ semantics),
//!   with tombstones so iteration order survives deletions.
//! * `Set<T>`: the same table without values. Iteration is insertion
//!   order (CPython iterates in hash order; documented divergence).
//! * `Tup<T>`: an immutable, variable-length homogeneous tuple
//!   (`tuple[T, ...]`). Fixed-shape tuples are plain Rust tuples.
//! * `Never`: the element type of a container no element was ever added to.
//!
//! Every operation borrows the `RefCell` only for its own duration, so
//! user code never runs while a borrow is held.

use crate::conv::PyFmt;
use crate::exc::{Exc, PyResult};
use crate::{Str, Truthy};
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::hash::Hash;
use std::rc::Rc;

// ---------------------------------------------------------------------------
// Element traits

/// Python `==` for values of one static type.
pub trait PyEq {
    fn py_eq(&self, other: &Self) -> bool;
}

/// Python ordering (`<` etc.) for values of one static type. `Err` is the
/// TypeError CPython raises for unorderable values.
pub trait PyOrd {
    fn py_cmp(&self, other: &Self) -> PyResult<Ordering>;
}

macro_rules! eq_by_partial_eq {
    ($($t:ty),*) => {$(
        impl PyEq for $t {
            #[inline]
            fn py_eq(&self, o: &Self) -> bool { self == o }
        }
    )*};
}
eq_by_partial_eq!(i64, f64, bool, (), Str);

impl PyOrd for i64 {
    fn py_cmp(&self, o: &Self) -> PyResult<Ordering> {
        Ok(self.cmp(o))
    }
}

impl PyOrd for bool {
    fn py_cmp(&self, o: &Self) -> PyResult<Ordering> {
        Ok(self.cmp(o))
    }
}

impl PyOrd for f64 {
    fn py_cmp(&self, o: &Self) -> PyResult<Ordering> {
        // NaN compares false both ways; treat it as equal for sorting.
        Ok(self.partial_cmp(o).unwrap_or(Ordering::Equal))
    }
}

impl PyOrd for Str {
    fn py_cmp(&self, o: &Self) -> PyResult<Ordering> {
        // UTF-8 byte order is code point order.
        Ok(self.as_bytes().cmp(o.as_bytes()))
    }
}

impl PyOrd for () {
    fn py_cmp(&self, _: &Self) -> PyResult<Ordering> {
        Err(Exc::new("TypeError", "'<' not supported between instances of 'NoneType' and 'NoneType'"))
    }
}

impl<T: PyEq> PyEq for Option<T> {
    fn py_eq(&self, o: &Self) -> bool {
        match (self, o) {
            (Some(a), Some(b)) => a.py_eq(b),
            (None, None) => true,
            _ => false,
        }
    }
}

impl<T: PyOrd> PyOrd for Option<T> {
    fn py_cmp(&self, o: &Self) -> PyResult<Ordering> {
        match (self, o) {
            (Some(a), Some(b)) => a.py_cmp(b),
            _ => Err(Exc::new("TypeError", "'<' not supported between instances of 'NoneType' and another value")),
        }
    }
}

impl PyEq for Exc {
    fn py_eq(&self, o: &Self) -> bool {
        self == o
    }
}

/// `a < b` from `py_cmp`.
pub fn lt<T: PyOrd>(a: &T, b: &T) -> PyResult<bool> {
    Ok(a.py_cmp(b)? == Ordering::Less)
}
pub fn le<T: PyOrd>(a: &T, b: &T) -> PyResult<bool> {
    Ok(a.py_cmp(b)? != Ordering::Greater)
}
pub fn gt<T: PyOrd>(a: &T, b: &T) -> PyResult<bool> {
    Ok(a.py_cmp(b)? == Ordering::Greater)
}
pub fn ge<T: PyOrd>(a: &T, b: &T) -> PyResult<bool> {
    Ok(a.py_cmp(b)? != Ordering::Less)
}
pub fn eq<T: PyEq>(a: &T, b: &T) -> bool {
    a.py_eq(b)
}
pub fn ne<T: PyEq>(a: &T, b: &T) -> bool {
    !a.py_eq(b)
}

// ---------------------------------------------------------------------------
// Never: element type of a container that never received an element.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Never {}

impl PyEq for Never {
    fn py_eq(&self, _: &Self) -> bool {
        match *self {}
    }
}
impl PyOrd for Never {
    fn py_cmp(&self, _: &Self) -> PyResult<Ordering> {
        match *self {}
    }
}
impl PyFmt for Never {
    fn py_repr(&self) -> Str {
        match *self {}
    }
}
impl Truthy for Never {
    fn truthy(&self) -> bool {
        match *self {}
    }
}

// ---------------------------------------------------------------------------
// Index helpers

/// Normalise a Python index (negative counts from the end).
#[inline]
fn norm(i: i64, len: usize) -> Option<usize> {
    let len = len as i64;
    let j = if i < 0 { i + len } else { i };
    (0..len).contains(&j).then_some(j as usize)
}

/// Python's `slice.indices(len)`: (start, stop, step) clamped like CPython.
pub fn slice_indices(start: Option<i64>, stop: Option<i64>, step: Option<i64>, len: usize) -> PyResult<(i64, i64, i64)> {
    let len = len as i64;
    let step = step.unwrap_or(1);
    if step == 0 {
        return Err(Exc::new("ValueError", "slice step cannot be zero"));
    }
    let clamp = |v: i64, lo: i64, hi: i64| v.max(lo).min(hi);
    let adj = |v: i64, lo: i64, hi: i64| if v < 0 { clamp(v + len, lo, hi) } else { clamp(v, lo, hi) };
    let (start, stop) = if step > 0 {
        (start.map(|v| adj(v, 0, len)).unwrap_or(0), stop.map(|v| adj(v, 0, len)).unwrap_or(len))
    } else {
        (start.map(|v| adj(v, -1, len - 1)).unwrap_or(len - 1), stop.map(|v| adj(v, -1, len - 1)).unwrap_or(-1))
    };
    Ok((start, stop, step))
}

/// The positions selected by a slice.
pub fn slice_positions(start: Option<i64>, stop: Option<i64>, step: Option<i64>, len: usize) -> PyResult<Vec<usize>> {
    let (mut i, stop, step) = slice_indices(start, stop, step, len)?;
    let mut out = Vec::new();
    if step > 0 {
        while i < stop {
            out.push(i as usize);
            i += step;
        }
    } else {
        while i > stop {
            out.push(i as usize);
            i += step;
        }
    }
    Ok(out)
}

fn seq_repr<'a, T: PyFmt + 'a>(open: &str, items: impl Iterator<Item = &'a T>, close: &str) -> Str {
    let mut s = String::from(open);
    for (i, v) in items.enumerate() {
        if i > 0 {
            s.push_str(", ");
        }
        s.push_str(&v.py_repr());
    }
    s.push_str(close);
    Str::from(s)
}

fn seq_cmp<T: PyOrd>(a: &[T], b: &[T]) -> PyResult<Ordering> {
    for (x, y) in a.iter().zip(b) {
        match x.py_cmp(y)? {
            Ordering::Equal => continue,
            o => return Ok(o),
        }
    }
    Ok(a.len().cmp(&b.len()))
}

fn seq_eq<T: PyEq>(a: &[T], b: &[T]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.py_eq(y))
}

/// Stable sort with Python comparison semantics (first TypeError wins).
fn py_sort<T: PyOrd>(v: &mut [T], reverse: bool) -> PyResult<()> {
    let mut err = None;
    v.sort_by(|a, b| {
        if err.is_some() {
            return Ordering::Equal;
        }
        let r = if reverse { b.py_cmp(a) } else { a.py_cmp(b) };
        r.unwrap_or_else(|e| {
            err = Some(e);
            Ordering::Equal
        })
    });
    match err {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

fn not_in(v: &impl PyFmt, what: &str) -> Exc {
    Exc::new("ValueError", format!("{} is not in {what}", v.py_repr()))
}

// ---------------------------------------------------------------------------
// List

pub struct List<T>(Rc<RefCell<Vec<T>>>);

impl<T> Clone for List<T> {
    fn clone(&self) -> Self {
        List(self.0.clone())
    }
}

impl<T> Default for List<T> {
    fn default() -> Self {
        List::new()
    }
}

impl<T> List<T> {
    pub fn new() -> Self {
        List(Rc::new(RefCell::new(Vec::new())))
    }

    pub fn from_vec(v: Vec<T>) -> Self {
        List(Rc::new(RefCell::new(v)))
    }

    pub fn len(&self) -> i64 {
        self.0.borrow().len() as i64
    }

    pub fn is_empty(&self) -> bool {
        self.0.borrow().is_empty()
    }

    /// `a is b`
    pub fn is(&self, o: &Self) -> bool {
        Rc::ptr_eq(&self.0, &o.0)
    }

    pub fn append(&self, v: T) {
        self.0.borrow_mut().push(v);
    }

    pub fn clear(&self) {
        self.0.borrow_mut().clear();
    }

    pub fn reverse(&self) {
        self.0.borrow_mut().reverse();
    }

    pub fn insert(&self, i: i64, v: T) {
        let mut b = self.0.borrow_mut();
        let len = b.len() as i64;
        let j = if i < 0 { (i + len).max(0) } else { i.min(len) };
        b.insert(j as usize, v);
    }

    pub fn pop(&self, i: Option<i64>) -> PyResult<T> {
        let mut b = self.0.borrow_mut();
        if b.is_empty() {
            return Err(Exc::new("IndexError", "pop from empty list"));
        }
        let len = b.len();
        let j = norm(i.unwrap_or(-1), len).ok_or_else(|| Exc::new("IndexError", "pop index out of range"))?;
        Ok(b.remove(j))
    }

    pub fn del(&self, i: i64) -> PyResult<()> {
        let mut b = self.0.borrow_mut();
        let j = norm(i, b.len()).ok_or_else(|| Exc::new("IndexError", "list assignment index out of range"))?;
        b.remove(j);
        Ok(())
    }

    pub fn del_slice(&self, start: Option<i64>, stop: Option<i64>, step: Option<i64>) -> PyResult<()> {
        let mut b = self.0.borrow_mut();
        let mut pos = slice_positions(start, stop, step, b.len())?;
        pos.sort_unstable();
        for p in pos.into_iter().rev() {
            b.remove(p);
        }
        Ok(())
    }
}

impl<T: Clone> List<T> {
    /// Element `i` for iteration (index-based, so appends during a loop are
    /// seen, as in CPython).
    #[inline]
    pub fn at(&self, i: usize) -> Option<T> {
        self.0.borrow().get(i).cloned()
    }

    pub fn get(&self, i: i64) -> PyResult<T> {
        let b = self.0.borrow();
        norm(i, b.len()).map(|j| b[j].clone()).ok_or_else(|| Exc::new("IndexError", "list index out of range"))
    }

    pub fn set(&self, i: i64, v: T) -> PyResult<()> {
        let mut b = self.0.borrow_mut();
        let j = norm(i, b.len()).ok_or_else(|| Exc::new("IndexError", "list assignment index out of range"))?;
        b[j] = v;
        Ok(())
    }

    pub fn to_vec(&self) -> Vec<T> {
        self.0.borrow().clone()
    }

    pub fn copy(&self) -> List<T> {
        List::from_vec(self.to_vec())
    }

    pub fn slice(&self, start: Option<i64>, stop: Option<i64>, step: Option<i64>) -> PyResult<List<T>> {
        let b = self.0.borrow();
        let pos = slice_positions(start, stop, step, b.len())?;
        Ok(List::from_vec(pos.into_iter().map(|p| b[p].clone()).collect()))
    }

    /// `a[i:j] = items`
    pub fn set_slice(&self, start: Option<i64>, stop: Option<i64>, step: Option<i64>, items: Vec<T>) -> PyResult<()> {
        let mut b = self.0.borrow_mut();
        if step.unwrap_or(1) == 1 {
            let (s, e, _) = slice_indices(start, stop, None, b.len())?;
            let e = e.max(s);
            b.splice(s as usize..e as usize, items);
            return Ok(());
        }
        let pos = slice_positions(start, stop, step, b.len())?;
        if pos.len() != items.len() {
            return Err(Exc::new(
                "ValueError",
                format!("attempt to assign sequence of size {} to extended slice of size {}", items.len(), pos.len()),
            ));
        }
        for (p, v) in pos.into_iter().zip(items) {
            b[p] = v;
        }
        Ok(())
    }

    /// `a.extend(items)` / `a += items` (items are copied first, so `a += a` works).
    pub fn extend(&self, items: Vec<T>) {
        self.0.borrow_mut().extend(items);
    }

    /// `a + b`
    pub fn concat(&self, o: &List<T>) -> List<T> {
        let mut v = self.to_vec();
        v.extend(o.to_vec());
        List::from_vec(v)
    }

    /// `a * n`
    pub fn repeat(&self, n: i64) -> List<T> {
        let v = self.to_vec();
        let mut out = Vec::with_capacity(v.len() * n.max(0) as usize);
        for _ in 0..n.max(0) {
            out.extend(v.iter().cloned());
        }
        List::from_vec(out)
    }
}

impl<T: Clone + PyEq> List<T> {
    pub fn contains(&self, v: &T) -> bool {
        self.0.borrow().iter().any(|x| x.py_eq(v))
    }

    pub fn count(&self, v: &T) -> i64 {
        self.0.borrow().iter().filter(|x| x.py_eq(v)).count() as i64
    }
}

impl<T: Clone + PyEq + PyFmt> List<T> {
    pub fn index(&self, v: &T) -> PyResult<i64> {
        self.0.borrow().iter().position(|x| x.py_eq(v)).map(|p| p as i64).ok_or_else(|| not_in(v, "list"))
    }

    pub fn remove(&self, v: &T) -> PyResult<()> {
        let mut b = self.0.borrow_mut();
        match b.iter().position(|x| x.py_eq(v)) {
            Some(p) => {
                b.remove(p);
                Ok(())
            }
            None => Err(Exc::new("ValueError", "list.remove(x): x not in list")),
        }
    }
}

impl<T: Clone + PyOrd> List<T> {
    pub fn sort(&self, reverse: bool) -> PyResult<()> {
        // Sort a copy so a TypeError leaves the list as CPython would (unchanged order is close enough).
        let mut v = self.to_vec();
        py_sort(&mut v, reverse)?;
        *self.0.borrow_mut() = v;
        Ok(())
    }
}

impl<T: PyEq> PyEq for List<T> {
    fn py_eq(&self, o: &Self) -> bool {
        Rc::ptr_eq(&self.0, &o.0) || seq_eq(&self.0.borrow(), &o.0.borrow())
    }
}

impl<T: PyOrd> PyOrd for List<T> {
    fn py_cmp(&self, o: &Self) -> PyResult<Ordering> {
        seq_cmp(&self.0.borrow(), &o.0.borrow())
    }
}

impl<T: PyFmt> PyFmt for List<T> {
    fn py_repr(&self) -> Str {
        seq_repr("[", self.0.borrow().iter(), "]")
    }
}

impl<T> Truthy for List<T> {
    fn truthy(&self) -> bool {
        !self.0.borrow().is_empty()
    }
}

// ---------------------------------------------------------------------------
// Tup: tuple[T, ...]

pub struct Tup<T>(Rc<[T]>);

impl<T> Clone for Tup<T> {
    fn clone(&self) -> Self {
        Tup(self.0.clone())
    }
}

impl<T> Default for Tup<T> {
    fn default() -> Self {
        Tup(Rc::from(Vec::new()))
    }
}

impl<T: Clone> Tup<T> {
    pub fn from_vec(v: Vec<T>) -> Self {
        Tup(Rc::from(v))
    }
    pub fn len(&self) -> i64 {
        self.0.len() as i64
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn at(&self, i: usize) -> Option<T> {
        self.0.get(i).cloned()
    }
    pub fn get(&self, i: i64) -> PyResult<T> {
        norm(i, self.0.len()).map(|j| self.0[j].clone()).ok_or_else(|| Exc::new("IndexError", "tuple index out of range"))
    }
    pub fn to_vec(&self) -> Vec<T> {
        self.0.to_vec()
    }
    pub fn slice(&self, start: Option<i64>, stop: Option<i64>, step: Option<i64>) -> PyResult<Tup<T>> {
        let pos = slice_positions(start, stop, step, self.0.len())?;
        Ok(Tup::from_vec(pos.into_iter().map(|p| self.0[p].clone()).collect()))
    }
    pub fn concat(&self, o: &Tup<T>) -> Tup<T> {
        let mut v = self.to_vec();
        v.extend(o.to_vec());
        Tup::from_vec(v)
    }
}

impl<T: Clone + PyEq> Tup<T> {
    pub fn contains(&self, v: &T) -> bool {
        self.0.iter().any(|x| x.py_eq(v))
    }
    pub fn count(&self, v: &T) -> i64 {
        self.0.iter().filter(|x| x.py_eq(v)).count() as i64
    }
}

impl<T: Clone + PyEq + PyFmt> Tup<T> {
    pub fn index(&self, v: &T) -> PyResult<i64> {
        self.0.iter().position(|x| x.py_eq(v)).map(|p| p as i64).ok_or_else(|| not_in(v, "tuple"))
    }
}

impl<T: PyEq> PyEq for Tup<T> {
    fn py_eq(&self, o: &Self) -> bool {
        seq_eq(&self.0, &o.0)
    }
}
impl<T: PyOrd> PyOrd for Tup<T> {
    fn py_cmp(&self, o: &Self) -> PyResult<Ordering> {
        seq_cmp(&self.0, &o.0)
    }
}
impl<T: PyFmt> PyFmt for Tup<T> {
    fn py_repr(&self) -> Str {
        if self.0.len() == 1 {
            return Str::from(format!("({},)", self.0[0].py_repr()));
        }
        seq_repr("(", self.0.iter(), ")")
    }
}
impl<T> Truthy for Tup<T> {
    fn truthy(&self) -> bool {
        !self.0.is_empty()
    }
}
impl<T: Hash> Hash for Tup<T> {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        self.0.hash(h)
    }
}
impl<T: PartialEq> PartialEq for Tup<T> {
    fn eq(&self, o: &Self) -> bool {
        self.0 == o.0
    }
}
impl<T: Eq> Eq for Tup<T> {}

/// The empty tuple `()` (Rust's `()` is Python's `None`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Tuple0;

impl PyEq for Tuple0 {
    fn py_eq(&self, _: &Self) -> bool {
        true
    }
}
impl PyOrd for Tuple0 {
    fn py_cmp(&self, _: &Self) -> PyResult<Ordering> {
        Ok(Ordering::Equal)
    }
}
impl PyFmt for Tuple0 {
    fn py_repr(&self) -> Str {
        Str::from("()")
    }
}
impl Truthy for Tuple0 {
    fn truthy(&self) -> bool {
        false
    }
}

// Fixed-shape tuples are Rust tuples.
macro_rules! tuple_impls {
    ($($n:tt $T:ident),+) => {
        impl<$($T: PyEq),+> PyEq for ($($T,)+) {
            fn py_eq(&self, o: &Self) -> bool { true $(&& self.$n.py_eq(&o.$n))+ }
        }
        impl<$($T: PyOrd),+> PyOrd for ($($T,)+) {
            fn py_cmp(&self, o: &Self) -> PyResult<Ordering> {
                $(match self.$n.py_cmp(&o.$n)? { Ordering::Equal => {}, c => return Ok(c) })+
                Ok(Ordering::Equal)
            }
        }
        impl<$($T: PyFmt),+> PyFmt for ($($T,)+) {
            fn py_repr(&self) -> Str {
                let parts: Vec<String> = vec![$(self.$n.py_repr().to_string()),+];
                if parts.len() == 1 {
                    Str::from(format!("({},)", parts[0]))
                } else {
                    Str::from(format!("({})", parts.join(", ")))
                }
            }
        }
        impl<$($T),+> Truthy for ($($T,)+) {
            fn truthy(&self) -> bool { true }
        }
    };
}
tuple_impls!(0 A);
tuple_impls!(0 A, 1 B);
tuple_impls!(0 A, 1 B, 2 C);
tuple_impls!(0 A, 1 B, 2 C, 3 D);
tuple_impls!(0 A, 1 B, 2 C, 3 D, 4 E);
tuple_impls!(0 A, 1 B, 2 C, 3 D, 4 E, 5 F);
tuple_impls!(0 A, 1 B, 2 C, 3 D, 4 E, 5 F, 6 G);
tuple_impls!(0 A, 1 B, 2 C, 3 D, 4 E, 5 F, 6 G, 7 H);
tuple_impls!(0 A, 1 B, 2 C, 3 D, 4 E, 5 F, 6 G, 7 H, 8 I);
tuple_impls!(0 A, 1 B, 2 C, 3 D, 4 E, 5 F, 6 G, 7 H, 8 I, 9 J);
tuple_impls!(0 A, 1 B, 2 C, 3 D, 4 E, 5 F, 6 G, 7 H, 8 I, 9 J, 10 K);
tuple_impls!(0 A, 1 B, 2 C, 3 D, 4 E, 5 F, 6 G, 7 H, 8 I, 9 J, 10 K, 11 L);

// ---------------------------------------------------------------------------
// Ordered table shared by Dict and Set

/// FxHash (rustc's hasher): much faster than SipHash for the short keys
/// programs use. Not DoS-resistant, like CPython's int hashing.
#[derive(Default, Clone, Copy)]
pub struct FxHasher(u64);

const FX_K: u64 = 0x517c_c1b7_2722_0a95;

impl std::hash::Hasher for FxHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.chunks_exact(8);
        for c in &mut chunks {
            self.add(u64::from_le_bytes(c.try_into().unwrap()));
        }
        let rest = chunks.remainder();
        if !rest.is_empty() {
            let mut buf = [0u8; 8];
            buf[..rest.len()].copy_from_slice(rest);
            self.add(u64::from_le_bytes(buf));
        }
    }
    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.add(i as u64);
    }
    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.add(i);
    }
    #[inline]
    fn write_i64(&mut self, i: i64) {
        self.add(i as u64);
    }
    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }
}

impl FxHasher {
    #[inline]
    fn add(&mut self, w: u64) {
        self.0 = (self.0.rotate_left(5) ^ w).wrapping_mul(FX_K);
    }
}

type FxBuild = std::hash::BuildHasherDefault<FxHasher>;

struct Table<K, V> {
    entries: Vec<Option<(K, V)>>,
    index: HashMap<K, usize, FxBuild>,
}

impl<K: Clone + Eq + Hash, V> Table<K, V> {
    fn new() -> Self {
        Table { entries: Vec::new(), index: HashMap::default() }
    }

    fn len(&self) -> usize {
        self.index.len()
    }

    fn get(&self, k: &K) -> Option<&V> {
        self.index.get(k).and_then(|&i| self.entries[i].as_ref().map(|e| &e.1))
    }

    /// Insert or overwrite (an existing key keeps its position).
    fn insert(&mut self, k: K, v: V) {
        match self.index.get(&k) {
            Some(&i) => self.entries[i].as_mut().unwrap().1 = v,
            None => {
                self.index.insert(k.clone(), self.entries.len());
                self.entries.push(Some((k, v)));
            }
        }
    }

    fn remove(&mut self, k: &K) -> Option<V> {
        let i = self.index.remove(k)?;
        let v = self.entries[i].take().map(|e| e.1);
        if self.entries.len() > 32 && self.index.len() * 2 < self.entries.len() {
            self.compact();
        }
        v
    }

    fn compact(&mut self) {
        self.entries.retain(|e| e.is_some());
        for (i, e) in self.entries.iter().enumerate() {
            let k = &e.as_ref().unwrap().0;
            *self.index.get_mut(k).unwrap() = i;
        }
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.index.clear();
    }

    fn iter(&self) -> impl Iterator<Item = &(K, V)> {
        self.entries.iter().flatten()
    }

    /// Next live entry at or after `pos`: (entry position, entry).
    fn next_from(&self, mut pos: usize) -> Option<(usize, &(K, V))> {
        while pos < self.entries.len() {
            if let Some(e) = &self.entries[pos] {
                return Some((pos, e));
            }
            pos += 1;
        }
        None
    }

    fn pop_last(&mut self) -> Option<(K, V)> {
        while let Some(e) = self.entries.pop() {
            if let Some((k, v)) = e {
                self.index.remove(&k);
                return Some((k, v));
            }
        }
        None
    }

    fn pop_first(&mut self) -> Option<(K, V)> {
        let (pos, _) = self.next_from(0)?;
        let (k, v) = self.entries[pos].take()?;
        self.index.remove(&k);
        Some((k, v))
    }
}

// ---------------------------------------------------------------------------
// Dict

pub struct Dict<K, V>(Rc<RefCell<Table<K, V>>>);

impl<K, V> Clone for Dict<K, V> {
    fn clone(&self) -> Self {
        Dict(self.0.clone())
    }
}

impl<K: Clone + Eq + Hash, V: Clone> Default for Dict<K, V> {
    fn default() -> Self {
        Dict::new()
    }
}

fn key_error(k: &impl PyFmt) -> Exc {
    Exc::new("KeyError", k.py_repr().to_string())
}

impl<K: Clone + Eq + Hash, V: Clone> Dict<K, V> {
    pub fn new() -> Self {
        Dict(Rc::new(RefCell::new(Table::new())))
    }

    pub fn from_pairs(pairs: Vec<(K, V)>) -> Self {
        let d = Dict::new();
        for (k, v) in pairs {
            d.set(k, v);
        }
        d
    }

    pub fn is(&self, o: &Self) -> bool {
        Rc::ptr_eq(&self.0, &o.0)
    }

    pub fn len(&self) -> i64 {
        self.0.borrow().len() as i64
    }

    pub fn is_empty(&self) -> bool {
        self.0.borrow().len() == 0
    }

    pub fn set(&self, k: K, v: V) {
        self.0.borrow_mut().insert(k, v);
    }

    pub fn contains(&self, k: &K) -> bool {
        self.0.borrow().index.contains_key(k)
    }

    /// `d.get(k)` / `d.get(k, default)`
    pub fn get_or(&self, k: &K, default: V) -> V {
        self.0.borrow().get(k).cloned().unwrap_or(default)
    }

    /// `d.get(k)` with no default: `None` when missing.
    pub fn get_opt(&self, k: &K) -> Option<V> {
        self.0.borrow().get(k).cloned()
    }

    pub fn clear(&self) {
        self.0.borrow_mut().clear();
    }

    pub fn copy(&self) -> Dict<K, V> {
        Dict::from_pairs(self.items_vec())
    }

    pub fn keys_vec(&self) -> Vec<K> {
        self.0.borrow().iter().map(|e| e.0.clone()).collect()
    }

    pub fn values_vec(&self) -> Vec<V> {
        self.0.borrow().iter().map(|e| e.1.clone()).collect()
    }

    pub fn items_vec(&self) -> Vec<(K, V)> {
        self.0.borrow().iter().cloned().collect()
    }

    /// `d.update(other)`
    pub fn update(&self, o: &Dict<K, V>) {
        for (k, v) in o.items_vec() {
            self.set(k, v);
        }
    }

    /// `d | other`
    pub fn merge(&self, o: &Dict<K, V>) -> Dict<K, V> {
        let d = self.copy();
        d.update(o);
        d
    }

    /// Iteration cursor (raises RuntimeError if the size changes).
    pub fn cursor(&self) -> DictCursor<K, V> {
        DictCursor { d: self.clone(), pos: 0, len: self.0.borrow().len() }
    }
}

impl<K: Clone + Eq + Hash + PyFmt, V: Clone> Dict<K, V> {
    /// `d[k]`
    pub fn get(&self, k: &K) -> PyResult<V> {
        self.0.borrow().get(k).cloned().ok_or_else(|| key_error(k))
    }

    /// `del d[k]`
    pub fn del(&self, k: &K) -> PyResult<()> {
        self.0.borrow_mut().remove(k).map(|_| ()).ok_or_else(|| key_error(k))
    }

    /// `d.pop(k)`
    pub fn pop(&self, k: &K) -> PyResult<V> {
        self.0.borrow_mut().remove(k).ok_or_else(|| key_error(k))
    }

    /// `d.pop(k, default)`
    pub fn pop_or(&self, k: &K, default: V) -> V {
        self.0.borrow_mut().remove(k).unwrap_or(default)
    }

    /// `d.setdefault(k, default)`
    pub fn setdefault(&self, k: K, default: V) -> V {
        if let Some(v) = self.0.borrow().get(&k) {
            return v.clone();
        }
        self.set(k, default.clone());
        default
    }

    /// `d.popitem()` (last inserted)
    pub fn popitem(&self) -> PyResult<(K, V)> {
        self.0.borrow_mut().pop_last().ok_or_else(|| Exc::new("KeyError", "'popitem(): dictionary is empty'"))
    }
}

/// Cursor over a dict's entries in insertion order.
pub struct DictCursor<K, V> {
    d: Dict<K, V>,
    pos: usize,
    len: usize,
}

impl<K: Clone + Eq + Hash, V: Clone> DictCursor<K, V> {
    pub fn next(&mut self) -> PyResult<Option<(K, V)>> {
        let t = self.d.0.borrow();
        if t.len() != self.len {
            return Err(Exc::new("RuntimeError", "dictionary changed size during iteration"));
        }
        match t.next_from(self.pos) {
            Some((p, e)) => {
                self.pos = p + 1;
                Ok(Some(e.clone()))
            }
            None => Ok(None),
        }
    }
}

impl<K: Clone + Eq + Hash, V: Clone + PyEq> PyEq for Dict<K, V> {
    fn py_eq(&self, o: &Self) -> bool {
        if Rc::ptr_eq(&self.0, &o.0) {
            return true;
        }
        let (a, b) = (self.0.borrow(), o.0.borrow());
        a.len() == b.len() && a.iter().all(|(k, v)| b.get(k).is_some_and(|w| v.py_eq(w)))
    }
}

impl<K: PyFmt, V: PyFmt> PyFmt for Dict<K, V> {
    fn py_repr(&self) -> Str {
        let t = self.0.borrow();
        let mut s = String::from("{");
        for (i, (k, v)) in t.entries.iter().flatten().enumerate() {
            if i > 0 {
                s.push_str(", ");
            }
            s.push_str(&k.py_repr());
            s.push_str(": ");
            s.push_str(&v.py_repr());
        }
        s.push('}');
        Str::from(s)
    }
}

impl<K, V> Truthy for Dict<K, V> {
    fn truthy(&self) -> bool {
        !self.0.borrow().entries.iter().all(|e| e.is_none())
    }
}

// ---------------------------------------------------------------------------
// Set

pub struct Set<T>(Rc<RefCell<Table<T, ()>>>);

impl<T> Clone for Set<T> {
    fn clone(&self) -> Self {
        Set(self.0.clone())
    }
}

impl<T: Clone + Eq + Hash> Default for Set<T> {
    fn default() -> Self {
        Set::new()
    }
}

impl<T: Clone + Eq + Hash> Set<T> {
    pub fn new() -> Self {
        Set(Rc::new(RefCell::new(Table::new())))
    }

    pub fn from_vec(v: Vec<T>) -> Self {
        let s = Set::new();
        for x in v {
            s.add(x);
        }
        s
    }

    pub fn is(&self, o: &Self) -> bool {
        Rc::ptr_eq(&self.0, &o.0)
    }

    pub fn len(&self) -> i64 {
        self.0.borrow().len() as i64
    }

    pub fn is_empty(&self) -> bool {
        self.0.borrow().len() == 0
    }

    pub fn add(&self, v: T) {
        let mut t = self.0.borrow_mut();
        if !t.index.contains_key(&v) {
            t.insert(v, ());
        }
    }

    pub fn contains(&self, v: &T) -> bool {
        self.0.borrow().index.contains_key(v)
    }

    pub fn discard(&self, v: &T) {
        self.0.borrow_mut().remove(v);
    }

    pub fn clear(&self) {
        self.0.borrow_mut().clear();
    }

    pub fn to_vec(&self) -> Vec<T> {
        self.0.borrow().iter().map(|e| e.0.clone()).collect()
    }

    pub fn copy(&self) -> Set<T> {
        Set::from_vec(self.to_vec())
    }

    pub fn update(&self, items: Vec<T>) {
        for x in items {
            self.add(x);
        }
    }

    pub fn union(&self, o: &Set<T>) -> Set<T> {
        let s = self.copy();
        s.update(o.to_vec());
        s
    }

    pub fn intersection(&self, o: &Set<T>) -> Set<T> {
        Set::from_vec(self.to_vec().into_iter().filter(|x| o.contains(x)).collect())
    }

    pub fn difference(&self, o: &Set<T>) -> Set<T> {
        Set::from_vec(self.to_vec().into_iter().filter(|x| !o.contains(x)).collect())
    }

    pub fn symmetric_difference(&self, o: &Set<T>) -> Set<T> {
        let mut v: Vec<T> = self.to_vec().into_iter().filter(|x| !o.contains(x)).collect();
        v.extend(o.to_vec().into_iter().filter(|x| !self.contains(x)));
        Set::from_vec(v)
    }

    pub fn issubset(&self, o: &Set<T>) -> bool {
        self.to_vec().iter().all(|x| o.contains(x))
    }

    pub fn issuperset(&self, o: &Set<T>) -> bool {
        o.issubset(self)
    }

    pub fn isdisjoint(&self, o: &Set<T>) -> bool {
        self.to_vec().iter().all(|x| !o.contains(x))
    }

    pub fn cursor(&self) -> SetCursor<T> {
        SetCursor { s: self.clone(), pos: 0, len: self.0.borrow().len() }
    }
}

impl<T: Clone + Eq + Hash + PyFmt> Set<T> {
    pub fn remove(&self, v: &T) -> PyResult<()> {
        self.0.borrow_mut().remove(v).ok_or_else(|| key_error(v))
    }

    pub fn pop(&self) -> PyResult<T> {
        self.0.borrow_mut().pop_first().map(|e| e.0).ok_or_else(|| Exc::new("KeyError", "'pop from an empty set'"))
    }
}

pub struct SetCursor<T> {
    s: Set<T>,
    pos: usize,
    len: usize,
}

impl<T: Clone + Eq + Hash> SetCursor<T> {
    pub fn next(&mut self) -> PyResult<Option<T>> {
        let t = self.s.0.borrow();
        if t.len() != self.len {
            return Err(Exc::new("RuntimeError", "Set changed size during iteration"));
        }
        match t.next_from(self.pos) {
            Some((p, e)) => {
                self.pos = p + 1;
                Ok(Some(e.0.clone()))
            }
            None => Ok(None),
        }
    }
}

impl<T: Clone + Eq + Hash> PyEq for Set<T> {
    fn py_eq(&self, o: &Self) -> bool {
        self.len() == o.len() && self.issubset(o)
    }
}

impl<T: PyFmt> PyFmt for Set<T> {
    fn py_repr(&self) -> Str {
        let t = self.0.borrow();
        if t.entries.iter().all(|e| e.is_none()) {
            return Str::from("set()");
        }
        seq_repr("{", t.entries.iter().flatten().map(|e| &e.0), "}")
    }
}

impl<T> Truthy for Set<T> {
    fn truthy(&self) -> bool {
        !self.0.borrow().entries.iter().all(|e| e.is_none())
    }
}

// ---------------------------------------------------------------------------
// Builtins over sequences

/// `sorted(items, reverse=...)`
pub fn sorted<T: PyOrd>(mut v: Vec<T>, reverse: bool) -> PyResult<List<T>> {
    py_sort(&mut v, reverse)?;
    Ok(List::from_vec(v))
}

/// `min(items)` / `max(items)`: the first extreme element.
pub fn min_of<T: PyOrd + PyFmt>(v: Vec<T>, want_max: bool) -> PyResult<T> {
    let mut it = v.into_iter();
    let mut best = it.next().ok_or_else(|| {
        Exc::new("ValueError", format!("{}() iterable argument is empty", if want_max { "max" } else { "min" }))
    })?;
    for x in it {
        let better = if want_max { x.py_cmp(&best)? == Ordering::Greater } else { x.py_cmp(&best)? == Ordering::Less };
        if better {
            best = x;
        }
    }
    Ok(best)
}

/// `sum(ints)`
pub fn sum_int(v: Vec<i64>, start: i64) -> PyResult<i64> {
    v.into_iter().try_fold(start, crate::int::add)
}

/// `sum(floats)` (CPython uses compensated summation for floats).
pub fn sum_float(v: Vec<f64>, start: f64) -> f64 {
    // Neumaier summation, matching CPython 3.12's float sum.
    let mut s = start;
    let mut c = 0.0;
    for x in v {
        let t = s + x;
        if s.abs() >= x.abs() {
            c += (s - t) + x;
        } else {
            c += (x - t) + s;
        }
        s = t;
    }
    s + c
}

/// Characters of a str, for iteration.
pub fn chars(s: &Str) -> Vec<Str> {
    s.chars().map(|c| Str::from(c.to_string())).collect()
}

/// `range(start, stop, step)` as a vector (for `list(range(...))` and friends).
pub fn range_vec(start: i64, stop: i64, step: i64) -> PyResult<Vec<i64>> {
    Ok(crate::int::range(start, stop, step)?.collect())
}

/// `any(...)` / `all(...)`
pub fn any<T: Truthy>(v: &[T]) -> bool {
    v.iter().any(|x| x.truthy())
}
pub fn all<T: Truthy>(v: &[T]) -> bool {
    v.iter().all(|x| x.truthy())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_semantics() {
        let a: List<i64> = List::from_vec(vec![3, 1, 2]);
        let b = a.clone();
        b.append(5);
        assert_eq!(a.len(), 4);
        assert_eq!(a.get(-1).unwrap(), 5);
        assert!(a.get(9).is_err());
        a.sort(false).unwrap();
        assert_eq!(&*a.py_repr(), "[1, 2, 3, 5]");
        assert_eq!(&*a.slice(None, None, Some(-1)).unwrap().py_repr(), "[5, 3, 2, 1]");
        assert_eq!(&*a.slice(Some(1), Some(-1), None).unwrap().py_repr(), "[2, 3]");
        a.set_slice(Some(1), Some(3), None, vec![9]).unwrap();
        assert_eq!(&*a.py_repr(), "[1, 9, 5]");
        assert_eq!(a.pop(None).unwrap(), 5);
        assert_eq!(a.index(&9).unwrap(), 1);
        assert_eq!(a.index(&7).unwrap_err().msg, "7 is not in list");
    }

    #[test]
    fn dict_order_and_errors() {
        let d: Dict<Str, i64> = Dict::new();
        d.set(Str::from("b"), 1);
        d.set(Str::from("a"), 2);
        d.set(Str::from("b"), 3);
        assert_eq!(&*d.py_repr(), "{'b': 3, 'a': 2}");
        assert_eq!(d.get(&Str::from("z")).unwrap_err().to_string(), "KeyError: 'z'");
        d.del(&Str::from("b")).unwrap();
        d.set(Str::from("b"), 4);
        assert_eq!(&*d.py_repr(), "{'a': 2, 'b': 4}");
        let mut c = d.cursor();
        assert!(c.next().unwrap().is_some());
        d.set(Str::from("c"), 5);
        assert!(c.next().is_err());
    }

    #[test]
    fn tuples_and_sets() {
        assert_eq!(&*(1i64, Str::from("x")).py_repr(), "(1, 'x')");
        assert_eq!(&*(1i64,).py_repr(), "(1,)");
        let s: Set<i64> = Set::from_vec(vec![3, 1, 3]);
        assert_eq!(s.len(), 2);
        assert_eq!(&*Set::<i64>::new().py_repr(), "set()");
        assert!((1i64, 2i64).py_cmp(&(1, 3)).unwrap() == Ordering::Less);
        assert_eq!(sum_float(vec![0.1; 10], 0.0), 1.0);
    }
}

/// Length check of an unpacking assignment (CPython's ValueError texts).
pub fn unpack_check(got: usize, n: usize, star: bool) -> PyResult<()> {
    if star {
        if got + 1 < n {
            return Err(Exc::new("ValueError", format!("not enough values to unpack (expected at least {}, got {got})", n - 1)));
        }
    } else if got > n {
        return Err(Exc::new("ValueError", format!("too many values to unpack (expected {n})")));
    } else if got < n {
        return Err(Exc::new("ValueError", format!("not enough values to unpack (expected {n}, got {got})")));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Dict views (live, like CPython's dict_keys / dict_values / dict_items)

macro_rules! dict_view {
    ($name:ident, $label:literal, $item:ty, |$e:ident| $get:expr) => {
        pub struct $name<K, V>(pub Dict<K, V>);

        impl<K, V> Clone for $name<K, V> {
            fn clone(&self) -> Self {
                $name(self.0.clone())
            }
        }

        impl<K: Clone + Eq + Hash, V: Clone> $name<K, V> {
            pub fn len(&self) -> i64 {
                self.0.len()
            }
            pub fn is_empty(&self) -> bool {
                self.0.is_empty()
            }
            pub fn to_vec(&self) -> Vec<$item> {
                self.0.items_vec().into_iter().map(|$e| $get).collect()
            }
            pub fn cursor(&self) -> ViewCursor<K, V, $item> {
                ViewCursor { c: self.0.cursor(), f: |$e| $get }
            }
        }

        impl<K: Clone + Eq + Hash + PyFmt, V: Clone + PyFmt> PyFmt for $name<K, V>
        where
            $item: PyFmt,
        {
            fn py_repr(&self) -> Str {
                let items = self.to_vec();
                let inner = seq_repr("[", items.iter(), "]");
                Str::from(format!("{}({})", $label, inner))
            }
        }

        impl<K: Clone + Eq + Hash, V: Clone> Truthy for $name<K, V> {
            fn truthy(&self) -> bool {
                !self.0.is_empty()
            }
        }
    };
}

dict_view!(Keys, "dict_keys", K, |e| e.0);
dict_view!(Values, "dict_values", V, |e| e.1);
dict_view!(Items, "dict_items", (K, V), |e| e);

impl<K: Clone + Eq + Hash, V: Clone> Keys<K, V> {
    pub fn contains(&self, k: &K) -> bool {
        self.0.contains(k)
    }
}

impl<K: Clone + Eq + Hash, V: Clone + PyEq> Values<K, V> {
    pub fn contains(&self, v: &V) -> bool {
        self.0.values_vec().iter().any(|x| x.py_eq(v))
    }
}

impl<K: Clone + Eq + Hash, V: Clone + PyEq> Items<K, V> {
    pub fn contains(&self, kv: &(K, V)) -> bool {
        self.0.get_opt(&kv.0).is_some_and(|v| v.py_eq(&kv.1))
    }
}

/// Live iteration over a dict view.
pub struct ViewCursor<K, V, T> {
    c: DictCursor<K, V>,
    f: fn((K, V)) -> T,
}

impl<K: Clone + Eq + Hash, V: Clone, T> ViewCursor<K, V, T> {
    pub fn next(&mut self) -> PyResult<Option<T>> {
        Ok(self.c.next()?.map(self.f))
    }
}

// ---------------------------------------------------------------------------
// Key functions (`sorted(xs, key=f)`, `max(xs, key=f)`, `xs.sort(key=f)`)

/// Decorate-sort-undecorate: keys are computed once, in order (a raising key
/// function stops at the same element as CPython), then a stable sort.
fn keyed_sort<T, K: PyOrd>(v: Vec<T>, mut key: impl FnMut(&T) -> PyResult<K>, reverse: bool) -> PyResult<Vec<T>> {
    let mut keyed: Vec<(K, T)> = Vec::with_capacity(v.len());
    for x in v {
        keyed.push((key(&x)?, x));
    }
    let mut err = None;
    keyed.sort_by(|a, b| {
        if err.is_some() {
            return Ordering::Equal;
        }
        let r = if reverse { b.0.py_cmp(&a.0) } else { a.0.py_cmp(&b.0) };
        r.unwrap_or_else(|e| {
            err = Some(e);
            Ordering::Equal
        })
    });
    match err {
        Some(e) => Err(e),
        None => Ok(keyed.into_iter().map(|(_, x)| x).collect()),
    }
}

pub fn sorted_by_key<T, K: PyOrd>(v: Vec<T>, key: impl FnMut(&T) -> PyResult<K>, reverse: bool) -> PyResult<List<T>> {
    Ok(List::from_vec(keyed_sort(v, key, reverse)?))
}

/// `min(xs, key=f)` / `max(xs, key=f)`: the first extreme element.
pub fn min_by_key<T, K: PyOrd>(v: Vec<T>, mut key: impl FnMut(&T) -> PyResult<K>, want_max: bool) -> PyResult<T> {
    let mut it = v.into_iter();
    let first = it.next().ok_or_else(|| Exc::new("ValueError", format!("{}() iterable argument is empty", if want_max { "max" } else { "min" })))?;
    let mut best_k = key(&first)?;
    let mut best = first;
    for x in it {
        let k = key(&x)?;
        let better = if want_max { k.py_cmp(&best_k)? == Ordering::Greater } else { k.py_cmp(&best_k)? == Ordering::Less };
        if better {
            best = x;
            best_k = k;
        }
    }
    Ok(best)
}

impl<T: Clone> List<T> {
    /// `xs.sort(key=f, reverse=r)`
    pub fn sort_by_key<K: PyOrd>(&self, key: impl FnMut(&T) -> PyResult<K>, reverse: bool) -> PyResult<()> {
        let v = keyed_sort(self.to_vec(), key, reverse)?;
        *self.0.borrow_mut() = v;
        Ok(())
    }
}
