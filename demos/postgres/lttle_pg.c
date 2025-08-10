#include "postgres.h"
#include "fmgr.h"
#include "executor/executor.h"
#include "postmaster/bgworker.h"
#include "storage/ipc.h"
#include <unistd.h>
#include <fcntl.h>
#include <errno.h>
#include <signal.h>

PG_MODULE_MAGIC;

PGDLLEXPORT void flash_ready_worker(Datum arg);

static ExecutorRun_hook_type onExecutorRunPrev;
static void onExecutorRun(QueryDesc *queryDesc, ScanDirection direction, uint64 count, bool execute_once);

static int lttle_fd = -1;

static int lttle_init(void);
static int lttle_send_cmd(const char *cmd);
static void flash_snapshot(void);
static void flash_lock(void);
static void flash_unlock(void);

#define FLASH_LOCK_CMD     "flash_lock"
#define FLASH_UNLOCK_CMD   "flash_unlock"
#define FLASH_SNAPSHOT_CMD "manual_trigger"

void _PG_init(void)
{
    BackgroundWorker worker;

    elog(LOG, "lttle_pg: init");

    if (lttle_init() < 0)
    {
        elog(LOG, "lttle_pg: failed to initialize lttle");
        return;
    }

    onExecutorRunPrev = ExecutorRun_hook;
	ExecutorRun_hook = onExecutorRun;

    MemSet(&worker, 0, sizeof(worker));

    worker.bgw_flags        = BGWORKER_SHMEM_ACCESS;
    worker.bgw_start_time   = BgWorkerStart_RecoveryFinished;
    worker.bgw_restart_time = BGW_NEVER_RESTART;

    snprintf(worker.bgw_name,          BGW_MAXLEN,  "flash-ready");
    snprintf(worker.bgw_library_name,  BGW_EXTRALEN, "/etc/lttle/lttle_pg.so");
    snprintf(worker.bgw_function_name, BGW_EXTRALEN, "flash_ready_worker");

    worker.bgw_main_arg = (Datum) 0;

    RegisterBackgroundWorker(&worker);
}

void _PG_fini(void)
{
    elog(LOG, "lttle_pg: fini");
}

void flash_ready_worker(Datum arg)
{    
    elog(LOG, "lttle_pg: flash ready worker");

    if (lttle_init() < 0)
    {
        elog(LOG, "lttle_pg: failed to initialize lttle");
        return;
    }

    flash_snapshot();
    proc_exit(0);
}

static void onExecutorRun(QueryDesc *queryDesc, ScanDirection direction, uint64 count, bool execute_once)
{
    flash_lock();

    PG_TRY();
    {
        if (onExecutorRunPrev)
        {
            (*onExecutorRunPrev)(queryDesc, direction, count, execute_once);
        }
        else
        {
            standard_ExecutorRun(queryDesc, direction, count, execute_once);
        }
    }
    
    PG_FINALLY();
    {
        flash_unlock();
    }

    PG_END_TRY();
}

static int lttle_init(void)
{
    lttle_fd = open("/proc/lttle", O_WRONLY);
    if (lttle_fd < 0)
        return -1;

    return lttle_fd;
}

static int lttle_send_cmd(const char *cmd)
{
    if (lttle_fd < 0)
        return -1;
    
    if (write(lttle_fd, cmd, strlen(cmd)) < 0)
        return -1;

    return 0;
}

static void flash_lock(void)
{
    lttle_send_cmd(FLASH_LOCK_CMD);
}

static void flash_unlock(void)
{
    lttle_send_cmd(FLASH_UNLOCK_CMD);
}

static void flash_snapshot(void)
{
    lttle_send_cmd(FLASH_SNAPSHOT_CMD);
}
