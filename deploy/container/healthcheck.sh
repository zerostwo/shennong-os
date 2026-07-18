#!/bin/sh
set -eu

curl --fail --silent --show-error http://127.0.0.1:3000/healthz >/dev/null
curl --fail --silent --show-error http://127.0.0.1:8081/healthz >/dev/null
curl --fail --silent --show-error http://127.0.0.1:8002/health >/dev/null
pg_isready -q -h 127.0.0.1 -p 5432 -U shennong_os -d shennong_os
