#!/usr/bin/env bash
# Fetch the raylib shared library this sample loads from lib/.
set -euo pipefail
cd "$(dirname "$0")"
bash ../fetch-raylib.sh lib
