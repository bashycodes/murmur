# Example environment for run.sh (copy, adjust paths; source it).
T=$HOME/.cache/flow-clone-agents-target-server-speed/release
export BIN=$T/server-speed-clean LIB=$T/libflow_llm.so
export MODEL=$HOME/models/qwen2.5-1.5b-instruct-q4_k_m.gguf
export CASES=/path/to/private/cases.jsonl SESSION=/path/to/private/session-server.json
export THREADS=4
