#!/usr/bin/env python3
"""Render a sanitized eval JSON result as one dependency-free static HTML report."""
import argparse, html, json
from pathlib import Path


def escape(value):
    return html.escape(str(value))


def render(report):
    results = report.get("results", [])
    passed = sum(bool(result.get("pass")) for result in results)
    total = len(results)
    score = round(100 * passed / total) if total else 0
    duration_ms = sum(result.get("duration_ms", 0) for result in results)
    tokens = sum((result.get("usage") or {}).get("total_tokens", 0) for result in results)
    complete = bool(report.get("complete", True))
    cards = []
    for result in results:
        state = "pass" if result.get("pass") else "fail"
        assertions = "".join(
            f'<li class="{("pass" if item["pass"] else "fail")}"><span>{escape(item["name"])}</span><strong>{("PASS" if item["pass"] else "FAIL")}</strong></li>'
            for item in result.get("assertions", [])
        )
        actions = result.get("actions", [])
        action_summary = " · ".join(filter(None, [
            f'{len(actions)} action{("s" if len(actions) != 1 else "")}',
            f'{result.get("model_invocations", 0)} model call{("s" if result.get("model_invocations", 0) != 1 else "")}',
            f'{result.get("duration_ms", 0) / 1000:.1f}s',
        ]))
        details = escape(json.dumps(result, indent=2, ensure_ascii=False))
        cards.append(
            f'<article class="fixture {state}"><header><div><span class="result-mark">{("✓" if state == "pass" else "×")}</span>'
            f'<h2>{escape(result.get("fixture_id", "unknown"))}</h2></div><span class="pill {state}">{state.upper()}</span></header>'
            f'<p class="fixture-meta">{escape(action_summary)}</p><ul class="assertions">{assertions}</ul>'
            f'<details><summary>Sanitized metrics</summary><pre>{details}</pre></details></article>'
        )
    provenance = escape(json.dumps(report.get("reproducibility", {}), indent=2, ensure_ascii=False))
    stop_reason = report.get("stop_reason")
    incomplete = "" if complete else f'<div class="notice">Incomplete run: {escape(stop_reason or "fixtures were skipped")}</div>'
    title = f'Habibi eval — {report.get("run_id", "unknown")}'
    return f'''<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>{escape(title)}</title>
<style>
:root{{--bg:#0c0d0e;--panel:#151719;--panel2:#1b1e20;--line:#2b3033;--text:#f2efe8;--muted:#9ea6a6;--accent:#d2a85a;--good:#63c58b;--bad:#ef7771}}
*{{box-sizing:border-box}}body{{margin:0;background:radial-gradient(circle at 85% -10%,#292416 0,transparent 32rem),var(--bg);color:var(--text);font:15px/1.5 ui-sans-serif,system-ui,sans-serif}}
main{{width:min(1120px,calc(100% - 32px));margin:auto;padding:52px 0 80px}}.eyebrow{{color:var(--accent);font-size:12px;font-weight:800;letter-spacing:.16em;text-transform:uppercase}}
h1{{font-size:clamp(34px,7vw,68px);line-height:1;margin:10px 0 16px;letter-spacing:-.045em}}.subtitle{{color:var(--muted);font-size:17px;margin:0 0 34px}}
.summary{{display:grid;grid-template-columns:repeat(5,1fr);gap:12px;margin:24px 0 32px}}.metric{{background:linear-gradient(145deg,var(--panel2),var(--panel));border:1px solid var(--line);border-radius:14px;padding:17px}}
.metric strong{{display:block;font-size:27px;letter-spacing:-.03em}}.metric span,.fixture-meta{{color:var(--muted);font-size:13px}}.score strong{{color:{("var(--good)" if passed == total and total else "var(--accent)")}}}
.progress{{height:7px;background:#24282a;border-radius:99px;overflow:hidden;margin-top:12px}}.progress i{{display:block;width:{score}%;height:100%;background:linear-gradient(90deg,var(--accent),var(--good))}}
.notice{{border:1px solid #76532c;background:#281f14;color:#efc77d;border-radius:12px;padding:12px 16px;margin-bottom:22px}}.section-title{{display:flex;justify-content:space-between;align-items:end;margin:38px 0 14px}}.section-title h2{{margin:0;font-size:24px}}
.fixtures{{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:14px}}.fixture{{background:var(--panel);border:1px solid var(--line);border-radius:16px;padding:19px;box-shadow:0 14px 35px #0004}}.fixture.pass{{border-top:3px solid var(--good)}}.fixture.fail{{border-top:3px solid var(--bad)}}
.fixture header{{display:flex;align-items:center;justify-content:space-between;gap:12px}}.fixture header>div{{display:flex;align-items:center;gap:10px;min-width:0}}.fixture h2{{font-size:18px;margin:0;overflow-wrap:anywhere}}.result-mark{{display:grid;place-items:center;width:25px;height:25px;border-radius:50%;background:#222;font-weight:900}}.fixture.pass .result-mark{{color:var(--good)}}.fixture.fail .result-mark{{color:var(--bad)}}
.pill{{font-size:10px;font-weight:900;letter-spacing:.09em;padding:4px 8px;border-radius:99px}}.pill.pass{{color:var(--good);background:#173326}}.pill.fail{{color:var(--bad);background:#3a1d1d}}.fixture-meta{{margin:10px 0 13px}}
.assertions{{list-style:none;padding:0;margin:0;border-top:1px solid var(--line)}}.assertions li{{display:flex;justify-content:space-between;gap:12px;padding:8px 2px;border-bottom:1px solid var(--line);font-size:13px}}.assertions .pass strong{{color:var(--good)}}.assertions .fail strong{{color:var(--bad)}}
details{{margin-top:13px}}summary{{cursor:pointer;color:var(--muted);font-size:13px}}pre{{white-space:pre-wrap;overflow-wrap:anywhere;background:#090a0b;border:1px solid var(--line);border-radius:10px;padding:14px;max-height:420px;overflow:auto;font:12px/1.5 ui-monospace,monospace}}
.provenance{{margin-top:28px;background:var(--panel);border:1px solid var(--line);border-radius:14px;padding:17px}}footer{{color:var(--muted);font-size:12px;margin-top:28px}}
@media(max-width:820px){{.summary{{grid-template-columns:repeat(2,1fr)}}.fixtures{{grid-template-columns:1fr}}}}@media(max-width:460px){{main{{width:min(100% - 20px,1120px);padding-top:30px}}.summary{{grid-template-columns:1fr 1fr}}.metric{{padding:13px}}}}
</style></head><body><main>
<div class="eyebrow">Habibi · event engine evaluation</div><h1>{passed}/{total} tasks passed</h1>
<p class="subtitle">{escape(report.get("provider"))} / {escape(report.get("model"))} · {escape(report.get("kind", "unknown"))} run · {escape(report.get("run_id", "unknown"))}</p>
{incomplete}<section class="summary">
<div class="metric score"><strong>{score}%</strong><span>pass rate</span><div class="progress"><i></i></div></div>
<div class="metric"><strong>{tokens:,}</strong><span>tokens</span></div><div class="metric"><strong>{duration_ms / 1000:.1f}s</strong><span>fixture time</span></div>
<div class="metric"><strong>${report.get("estimated_cost_usd", 0):.4f}</strong><span>estimated cost</span></div><div class="metric"><strong>{report.get("brave_searches", 0)}</strong><span>Brave searches</span></div>
</section><div class="section-title"><h2>Fixtures</h2><span class="eyebrow">{("complete" if complete else "incomplete")}</span></div>
<section class="fixtures">{''.join(cards)}</section>
<details class="provenance"><summary>Reproducibility and input hashes</summary><pre>{provenance}</pre></details>
<footer>Static, dependency-free report. Exact prompts, model responses, provider payloads, and traces remain in ignored local artifacts.</footer>
</main></body></html>'''


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("result", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    report = json.loads(args.result.read_text())
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(render(report))


if __name__ == "__main__":
    main()
