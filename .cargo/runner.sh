#!/usr/bin/env bash

BIN_PATH="$1"; shift

if [[ "$BIN_PATH" == */deps/* ]]; then
  # this preserves LD_LIBRARY_PATH, PATH, CARGO_HOME, etc.
  exec sudo -E -- "$BIN_PATH" "$@"
else
  exec "$BIN_PATH" "$@"
fi
