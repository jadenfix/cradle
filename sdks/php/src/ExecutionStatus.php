<?php

declare(strict_types=1);

namespace Beatbox;

/**
 * Terminal status of a synchronous execution.
 */
enum ExecutionStatus: string
{
    case Ok = 'ok';
    case Error = 'error';
    case Timeout = 'timeout';
    case Oom = 'oom';
    case Killed = 'killed';
    case Denied = 'denied';
}
