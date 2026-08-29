# GAIA benchmark harness

Runs the public 2023 GAIA validation split through Habibi's Chat HTTP API and scores the final
assistant message with GAIA's reference exact-answer scorer.

## Data

```sh
benchmarks/gaia/prepare.sh
```

This clones [`aymeric-roucher/GAIA`](https://github.com/aymeric-roucher/GAIA) into ignored
`.benchmarks/gaia/`. That repository contains the 165-task validation metadata, answers, and
attachments under Apache-2.0. The official Hugging Face dataset is gated; accept its terms and use
that source instead when preparing a formal submission.

## Search prerequisite

The local benchmark setup uses the Web Search extension with self-hosted SearXNG:

```sh
docker run -d --name habibi-searxng --restart unless-stopped \
  -p 127.0.0.1:18080:8080 \
  -v "$PWD/.benchmarks/searxng/settings.yml:/etc/searxng/settings.yml:ro" \
  searxng/searxng:latest
```

Set these in ignored `.env`:

```env
HABIBI_SEARCH_PROVIDER=searxng
HABIBI_SEARXNG_URL=http://127.0.0.1:18080
```

A local SearXNG trial initially returned results, then all default general-search engines became
suspended by rate limits, CAPTCHA, or timeouts. It is not a dependable no-key GAIA backend. Use a
Brave API key or a maintained SearXNG deployment before a formal web-enabled run; leave search
unconfigured otherwise so the unusable tool stays hidden.

## Smoke benchmark

```sh
mise exec -- cargo build
python benchmarks/gaia/run.py --level 1 --no-files --limit 5
```

Each task gets a fresh Habibi database and server process on port 18800. This isolates non-settling
action chains and makes timeouts recoverable. Results and full databases/logs are written beneath
ignored `.benchmarks/gaia-runs/`.

Attachment tasks receive an isolated Workspace grant. Process receives `/usr/bin/python3` only when
an attachment exists. Current Workspace is UTF-8-oriented and Python has no document-analysis
packages, so PDF, Office, image, audio, and video tasks are expected capability gaps rather than a
representative score. Start with no-file level-1 tasks.

## Initial diagnostic results

Using `gpt-5.6-luna` on the 2023 validation data:

- Terminal-action smoke task: **1/1** after fixing Chat to settle immediately after delivery.
- First five level-1 tasks without files: **1/5 (20%)**. Four runs ended with an empty provider
  response after tool discovery and therefore produced no chat answer.
- Five hand-selected no-file reasoning tasks: **3/5 (60%)**.

These are tiny diagnostic slices, not a GAIA score. They exposed and fixed a redundant Chat
acknowledgment loop, unreliable copying of opaque session UUIDs (now replaced by scoped `current`),
and repeated pruning telemetry. A configured search suggestion bypasses empty completions after
registry discovery, but the no-key SearXNG backend proved rate-limited. Remaining leading gaps are a
reliable search provider, bounded page retrieval, and PDF/Office/image/audio/video analysis tools.

Summarize one or more result files with:

```sh
python benchmarks/gaia/summarize.py .benchmarks/gaia-runs/*.jsonl
```

## Formal-run caveats

- GAIA is a live-web benchmark; results vary as sources change.
- Search snippets are not full browsing. Habibi currently lacks bounded page fetch and browser tools.
- The runner stores ground truth only in its evaluator process and never includes it in prompts or
  benchmark workspaces.
- Use the GAIA leaderboard's required submission format and current official scorer before claiming
  a comparable public score.
