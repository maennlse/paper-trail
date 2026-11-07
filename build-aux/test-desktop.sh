#!/bin/sh
set -eu
Xvfb :99 -screen 0 1600x1000x24 -nolisten tcp &
xvfb_pid=$!
trap 'kill "$xvfb_pid"' EXIT
export DISPLAY=:99
export GDK_BACKEND=x11
# Do not connect the isolated X server to the host's input-method service.
export GTK_IM_MODULE=gtk-im-context-simple
export PAPERTRAIL_INPUT_TESTS=1
# Xvfb needs to publish its display socket before GTK initializes.
for attempt in 1 2 3 4 5; do
  if [ -S /tmp/.X11-unix/X99 ]; then break; fi
  sleep 1
done
cargo test --workspace --locked -- --test-threads=1
