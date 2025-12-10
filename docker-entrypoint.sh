#!/bin/sh
set -e

mkdir -p "${CACHE_PATH:-/app/cache}"

if [ "${SKIP_INIT:-0}" != "1" ]; then
  /app/sayna init
fi

exec /app/sayna "$@"
