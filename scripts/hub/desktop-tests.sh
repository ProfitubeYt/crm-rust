#!/bin/sh
# Runs the desktop workspace's tests in the pinned Linux image built from
# desktop/tools/linux-dev.Dockerfile, with Cargo.lock as it is.
#
#   scripts/hub/desktop-tests.sh                              cargo test --workspace --locked
#   scripts/hub/desktop-tests.sh test -p tbs-core --locked epoch
#   scripts/hub/desktop-tests.sh check -p tbs-app --locked --target x86_64-pc-windows-gnu
#
# The image is built once; the registry and the build output live in the
# Docker volumes tbs-cargo-registry, tbs-cargo-git and tbs-target. Where a
# command may not outlive its shell, add -d after `docker run` by hand and
# read the result with `docker wait` and `docker logs`.
set -eu

root="$(cd "$(dirname "$0")/../.." && pwd)"
image="tbs-linux-dev:pinned"

docker image inspect "$image" >/dev/null 2>&1 \
    || docker build -f "$root/desktop/tools/linux-dev.Dockerfile" -t "$image" "$root/desktop/tools"

[ "$#" -gt 0 ] || set -- test --workspace --locked

exec docker run --rm \
    -v "$root":/work -v tbs-target:/target \
    -v tbs-cargo-registry:/usr/local/cargo/registry -v tbs-cargo-git:/usr/local/cargo/git \
    "$image" cargo "$@"
