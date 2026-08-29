#!/usr/bin/env python3
import argparse
import json
from collections import defaultdict
from pathlib import Path


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("results", type=Path, nargs="+")
    args = parser.parse_args()
    rows = []
    for path in args.results:
        rows.extend(json.loads(line) for line in path.read_text().splitlines() if line.strip())
    groups = defaultdict(list)
    for row in rows:
        groups[row["level"]].append(row)
    for label, selected in [("all", rows), *sorted(groups.items())]:
        total = len(selected)
        correct = sum(bool(row["correct"]) for row in selected)
        empty = sum(not row["prediction"] for row in selected)
        errors = sum(row["error"] is not None for row in selected)
        seconds = sum(row["elapsed_seconds"] for row in selected)
        tokens = sum((row.get("usage") or {}).get("total_tokens", 0) for row in selected)
        cost = sum((row.get("usage") or {}).get("estimated_cost_usd") or 0 for row in selected)
        print(
            f"{label}: score={correct}/{total} ({correct / total:.1%}) "
            f"empty={empty} errors={errors} avg_seconds={seconds / total:.1f} "
            f"tokens={tokens} estimated_cost_usd={cost:.4f}"
        )


if __name__ == "__main__":
    main()
