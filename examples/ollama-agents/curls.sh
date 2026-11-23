cd "$(dirname "$0")"
B=http://127.0.0.1:8090
run() { echo "\$ $*"; "$@" -sS -w '\n[HTTP %{http_code}]\n'; echo; }
run curl $B/health
run curl $B/agents
run curl -X POST $B/agents -d '{"name":"poet-calc","system":"You are a terse assistant. Always use calculate for arithmetic.","tools":["calculate","recall"]}'
run curl -X POST $B/agents -d '{"name":"poet-calc","tools":[]}'
run curl -X POST $B/agents -d '{"name":"bad","tools":["nope"]}'
run curl $B/agents
run curl -X POST $B/agents/poet-calc/chat -d '{"message":"What is 17 * 23 + 0.5?"}'
run curl -X POST $B/agents/note-taker/chat -d '{"message":"Remember that the wifi password is swordfish."}'
run curl -X POST $B/agents/text-analyst/chat -d '{"message":"Analyse: to be or not to be"}'
run curl -X POST $B/agents/ghost/chat -d '{"message":"hi"}'
run curl -X DELETE $B/agents/poet-calc
run curl -X DELETE $B/agents/poet-calc
run curl $B/agents
