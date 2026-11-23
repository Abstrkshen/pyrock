#!/usr/bin/env python3
"""Measure how much of a Python code base pyrock accepts natively.

Usage: python3 scripts/coverage.py [DIR] [--pyrock PATH] [--jobs N]

DIR defaults to the running interpreter's standard library. Each top-level `.py` file of
DIR is copied alone into a scratch directory (so its imports go to CPython instead of
compiling its siblings) and checked with `pyrock check` in hybrid mode. When DIR is a
package (it has `__init__.py`), every module in it is checked in place instead, imported
by a driver file, so relative imports resolve; CPython boundaries are not counted. A function or
method counts as accepted when no error is reported inside it and its class (if any) has
no class-level error. Prints totals and the most common blockers; see docs/coverage.md.

`check` type-checks only code reachable from the module body, so functions that the module
never calls are checked for supported constructs, not for types. The numbers are an upper
bound for "compiles natively" and a lower bound for "compiles in hybrid mode" (a module with
an error can still be imported through CPython).
"""
import argparse
import ast
import collections
import os
import re
import shutil
import subprocess
import sys
import sysconfig
import tempfile
from concurrent.futures import ThreadPoolExecutor

FUNCS = (ast.FunctionDef, ast.AsyncFunctionDef)
ERR = re.compile(r"\s*(\S+?):(\d+):\d+\s+(P\d+)\s+error: (.*)")


def check(pyrock, src, scratch):
    name = os.path.basename(src)[:-3]
    d = os.path.join(scratch, name)
    os.makedirs(d, exist_ok=True)
    shutil.copy(src, d)
    try:
        r = subprocess.run([pyrock, "check", os.path.join(d, name + ".py")],
                           capture_output=True, text=True, timeout=120)
        out = r.stdout + r.stderr
    except subprocess.TimeoutExpired:
        out = ""
    errs = []
    for line in out.splitlines():
        m = ERR.match(line)
        if m and m[1].endswith(name + ".py"):
            errs.append((int(m[2]), m[3], m[4]))
    return src, errs


def check_in_package(pyrock, src, pkg_root, pkg):
    """A module of a package, imported by a driver next to the package (so
    relative imports resolve). Native-only lists every error of the module;
    CPython boundaries (P2xx) are not blockers and are dropped."""
    rel = os.path.relpath(src, os.path.join(pkg_root, pkg))
    mod = rel[:-3].replace(os.sep, ".")
    dotted = pkg if mod == "__init__" else f"{pkg}." + mod.removesuffix(".__init__")
    driver = os.path.join(pkg_root, "_cov_" + dotted.replace(".", "_") + ".py")
    with open(driver, "w") as f:
        f.write(f"import {dotted}\n")
    try:
        r = subprocess.run([pyrock, "check", "--mode", "native-only", driver], capture_output=True, text=True, timeout=120)
        out = r.stdout + r.stderr
    except subprocess.TimeoutExpired:
        out = ""
    target = os.path.join(pkg, rel)
    errs = []
    for line in out.splitlines():
        m = ERR.match(line)
        if m and m[1].endswith(target) and not m[3].startswith("P2"):
            errs.append((int(m[2]), m[3], m[4]))
    return src, errs


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("dir", nargs="?", default=sysconfig.get_paths()["stdlib"])
    ap.add_argument("--pyrock", default=shutil.which("pyrock") or "pyrock")
    ap.add_argument("--jobs", type=int, default=os.cpu_count() or 4)
    a = ap.parse_args()

    scratch = tempfile.mkdtemp(prefix="pyrock-coverage-")
    if os.path.exists(os.path.join(a.dir, "__init__.py")):
        # A package: every module, checked in place (relative imports work).
        pkg = os.path.basename(os.path.normpath(a.dir))
        shutil.copytree(a.dir, os.path.join(scratch, pkg), ignore=shutil.ignore_patterns("__pycache__"))
        files = sorted(os.path.join(d, f) for d, _, fs in os.walk(os.path.join(scratch, pkg)) for f in fs if f.endswith(".py"))
        with ThreadPoolExecutor(a.jobs) as ex:
            results = list(ex.map(lambda f: check_in_package(a.pyrock, f, scratch, pkg), files))
    else:
        files = sorted(os.path.join(a.dir, f) for f in os.listdir(a.dir) if f.endswith(".py"))
        with ThreadPoolExecutor(a.jobs) as ex:
            results = list(ex.map(lambda f: check(a.pyrock, f, scratch), files))
    parsed = [(src, errs, open(src, encoding="utf-8", errors="replace").read()) for src, errs in results]
    shutil.rmtree(scratch, ignore_errors=True)

    n_files = clean_files = 0
    n_funcs = ok_funcs = lines = ok_lines = 0
    blockers = collections.Counter()
    for src, errs, text in parsed:
        try:
            tree = ast.parse(text)
        except (SyntaxError, UnicodeDecodeError):
            continue
        n_files += 1
        clean_files += not errs
        nodes = [x for x in ast.walk(tree) if isinstance(x, FUNCS + (ast.ClassDef,))]

        def owner(line):
            best = None
            for n in nodes:
                start = min([n.lineno] + [d.lineno for d in n.decorator_list])
                if start <= line <= n.end_lineno and (best is None or n.lineno >= best.lineno):
                    best = n
            return best

        bad = set()
        for line, code, msg in errs:
            if code != "P143":  # undefined names are mostly follow-ups of another blocker
                blockers[(code, re.sub(r"`[^`]*`", "X", msg)[:72])] += 1
            o = owner(line)
            if isinstance(o, ast.ClassDef):
                bad.update(id(f) for f in ast.walk(o) if isinstance(f, FUNCS))
            elif o is not None:
                bad.add(id(o))
        for f in (x for x in nodes if isinstance(x, FUNCS)):
            size = f.end_lineno - f.lineno + 1
            n_funcs += 1
            lines += size
            if id(f) not in bad:
                ok_funcs += 1
                ok_lines += size

    print(f"corpus: {a.dir} ({sys.version.split()[0]})")
    print(f"modules with no error:           {clean_files}/{n_files}")
    print(f"functions and methods accepted:  {ok_funcs}/{n_funcs} = {100 * ok_funcs / max(n_funcs, 1):.1f}%"
          f"  ({100 * ok_lines / max(lines, 1):.1f}% of their lines)")
    print("most common blockers:")
    for (code, msg), n in blockers.most_common(20):
        print(f"  {n:5}  {code}  {msg}")


if __name__ == "__main__":
    main()
