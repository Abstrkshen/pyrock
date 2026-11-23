# ollama-agents
An AI-agent server. The HTTP layer (server, Ollama client, JSON helpers) is a Rust module (`net.rs`, std + serde_json);
the agent logic is plain Python (`tools.py`, `agents.py`, `main.py`) compiled natively by pyrock with no CPython at
runtime. Agents (name, system prompt, tool list, model) are created at runtime and call tools through a local Ollama
`/api/chat` tool-calling loop. Default model `qwen3.5:4b` (any local model with tool support works via `--model`).
Needs Ollama on 127.0.0.1:11434.

<!-- readme_check: skip-ci (needs a local Ollama server with the qwen3.5:4b model) -->

```sh
pyrock check main.py --mode native-only
pyrock explain main.py
pyrock build main.py --release
ldd .pyrock/main/main | grep -i python   # prints nothing: libpython is not linked
pyrock run main.py --release -- --demo
pyrock run main.py --release -- --serve 8090 &
curl -s localhost:8090/health
curl -s localhost:8090/agents
curl -s -X POST localhost:8090/agents -d '{"name":"calc","system":"Use calculate for maths.","tools":["calculate"]}'
curl -s -X POST localhost:8090/agents/calc/chat -d '{"message":"What is 17 * 23 + 0.5?"}'
curl -s -X DELETE localhost:8090/agents/calc
kill %1
```

Files: `net.rs` Rust HTTP + JSON, `tools.py` calculate / word_stats / remember / recall, `agents.py` `Agent` + registry,
`main.py` routes and `--demo`. Recorded output: `demo-output.txt`, `serve-output.txt`, `explain.txt`; compiler
problems met while writing it: `FINDINGS.md` (minimal repros in `repro/`).
