#!/usr/bin/env bash

BIN_PATH="$1"; shift

# if the path contains the word "ignition" (lib tests) or "ignitiond" (daemon), we need to set the capabilities
if [[ "$BIN_PATH" == *"ignition"* || "$BIN_PATH" == *"ignitiond"* ]]; then
    echo "Setting capabilities for $BIN_PATH"
    sudo setcap 'cap_net_admin+ep cap_dac_override+ep cap_net_bind_service+ep' "$BIN_PATH"
fi

exec "$BIN_PATH" "$@"
