#!/usr/bin/env bash

BIN_PATH="$1"; shift
echo "Setting capabilities for $BIN_PATH"
sudo setcap 'cap_net_admin+ep cap_dac_override+ep' "$BIN_PATH"
exec "$BIN_PATH" "$@"
