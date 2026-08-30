# Habibi evals

Small, public-API eval harness for the durable event engine. It creates a fresh SQLite database, copied extension root, drafts directory, process, bind port, and (when configured) auth file per fixture. Raw databases and process logs stay under ignored `.benchmarks/habibi-evals/`; sanitized static reports go to `evals/reports/`.

## Deterministic run

```sh
cargo build
python3 -m unittest -v evals/test_eval.py
python3 evals/run.py
```

The default starts `fake_ollama.py`. It still exercises the model HTTP parser, per-event hooks, action groups, delivery modes, failures, Chat effects, durable inbox, and public trace/messages APIs. It uses no credentials, Brave calls, or provider spend.

## Live runs

Live network/model use is impossible unless `--live` is explicit:

```sh
python3 evals/run.py --live --provider ollama --model gemma4 --max-cost-usd 0 --max-brave-searches 20
python3 evals/run.py --live --provider openai-codex --model gpt-5.6-luna \
  --max-cost-usd 2 --reserve-per-fixture-usd .20 --max-brave-searches 20
```

OpenAI uses Habibi's Codex OAuth credential. If `HABIBI_AUTH_FILE` is set, the runner copies it per fixture so token refreshes do not mutate the source. Live OpenAI runs are rejected unless the selected model has catalog pricing. Cost enforcement is a soft observed ceiling: conservative reservation stops new fixtures, but only a provider-side quota can prevent one final response from overshooting. Each fixture declares its maximum expected Brave calls; use a quota-limited Brave key for a hard external billing boundary. Budget-stopped suites are recorded as incomplete and fail unless `--allow-partial` was explicit.

`fixtures.json` is the checked-in contract. Raw exact messages, provider responses, and traces stay only in ignored per-case `raw-result.json` files under `.benchmarks`. Public JSON/HTML contains summary assertions, normalized action identifiers/status, duration, provider/model, usage, estimated cost, and Brave count—never prompts, model payloads, or traces. Reports hash the binary, fixtures, harness, catalog, and copied extension inputs. A dirty-tree report is explicitly labeled non-reproducible from its Git SHA.

Render an existing result without running providers:

```sh
python3 evals/report.py evals/reports/<run>.json --output evals/reports/<run>.html
```

A report exists only after an actual deterministic or live run; `kind` labels which one produced it. Exact-answer and required-tool assertions are deliberately strict: a failed live report is useful model/runtime evidence, not a harness error.

## Checked-in reports

| Run | Result | Tokens | Time | Estimated cost |
| --- | ---: | ---: | ---: | ---: |
| [Deterministic engine suite](reports/20260830T203327Z.html) | 10/10 | 990 | 28.8s | $0 |
| [Ollama `gemma4`](reports/20260830T202510Z.html) | 3/8 | 37,241 | 166.2s | $0 |
| [OpenAI `gpt-5.6-luna`](reports/20260830T202512Z.html) | 2/8 | 30,509 | 256.9s | $0.007986 |

The static HTML files contain sanitized assertions and metrics. Exact prompts, results, provider payloads, and traces remain only in ignored local artifacts.
