#!/bin/bash
W=~/Claude/research/wispr/flow-clone/.claude/worktrees/core-p1
cd $W
for rep in 1 2; do
for s in np_false np_true pa_false pa_true np_false_long np_true_long pa_false_long pa_true_long; do
  echo "== $s rep $rep load $(cut -d' ' -f1 /proc/loadavg)"
  target/release/flow-cli transcribe --model models/ggml-tiny.en.bin --llm models/qwen2.5-1.5b-instruct-q4_k_m.gguf --threads 4 --realtime --session /tmp/flowbench/$s.json --context /tmp/flowbench/ctx.json models/jfk.wav 2>&1 | grep -E "load|llm last|ahead|^[A-Z]" | tail -4
done; done
