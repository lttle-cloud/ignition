#!/usr/bin/env bash

BIN_PATH="$1"; shift
sudo setcap cap_net_admin+ep "$BIN_PATH"
exec "$BIN_PATH" "$@"
