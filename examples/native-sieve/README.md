# native-sieve

Lists primes below 80 and counts primes below 3000. The whole program is native-only.

```sh
pyrock check main.py --mode native-only
pyrock explain main.py
pyrock run main.py
pyrock run main.py --release
pyrock bench main.py --baseline python --runs 5
```
