# ai: tokenizer + Rust model + C sampling

* `bpe.py`: a byte-pair-encoding tokenizer (train merges, encode, decode) in plain Python,
  compiled natively.
* `ngram.rs`: a bigram language model as a Rust struct (`Bigram.new`, `train(&[i64])`,
  `logits -> Vec<f64>`), using `HashMap` and slices. Imported with `import ngram`, no
  bindings.
* `sampling.c`: `softmax(double *x, int n)` (updates the list in place), `argmax`,
  `entropy`, linked with `-lm`.

The whole program compiles without CPython (`--mode native-only`).

```sh
pyrock check --mode native-only main.py
pyrock run main.py
pyrock bench bench_bpe.py --release --runs 3
```

`bench_bpe.py` trains the tokenizer on a generated 200k-word corpus. It is pure Python,
so CPython can run it too: about 3x faster natively (string-allocation bound).

`reference/` holds CPython versions of the Rust and C modules, used only to check the
output: `PYTHONPATH=reference python3 main.py` prints the same as `pyrock run main.py`.
