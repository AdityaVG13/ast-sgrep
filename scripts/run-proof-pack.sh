#!/usr/bin/env bash
# Runnable proof-pack gate (ghiw.5). Always writes COMPLIANCE_REPORT.md.
set -euo pipefail
cd "$(dirname "$0")/.."

status=0
python3 scripts/generate-compliance-report.py --tier proof-pack || status=$?
exit "$status"
