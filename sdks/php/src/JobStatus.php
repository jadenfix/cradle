<?php

declare(strict_types=1);

namespace Beatbox;

/**
 * Lifecycle status of an asynchronous job.
 */
enum JobStatus: string
{
    case Queued = 'queued';
    case Running = 'running';
    case Succeeded = 'succeeded';
    case Failed = 'failed';
    case Canceled = 'canceled';
}
