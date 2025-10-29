const fs = require('fs');

const FLASH_LOCK_CMD = "flash_lock";
const FLASH_UNLOCK_CMD = "flash_unlock";
const FLASH_SNAPSHOT_CMD = "manual_trigger";

function lttleControlWrite(command) {
    fs.writeFileSync('/proc/lttle', command);
}

function log(message) {
    console.log(`[lttle] ${message}`);
}
module.exports = {
    n8n: {
        ready: [
            async function () {
                log('n8n ready. trigger flash snapshot');
                lttleControlWrite(FLASH_SNAPSHOT_CMD);
            },
        ],
    },
    workflow: {
        preExecute: [
            async function () {
                log('workflow started. trigger flash lock');
                lttleControlWrite(FLASH_LOCK_CMD);
            },
        ],

        postExecute: [
            async function () {
                log('workflow finished. trigger flash unlock');
                lttleControlWrite(FLASH_UNLOCK_CMD);
            },
        ],
    },
};