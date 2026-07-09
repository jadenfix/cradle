"""beatbox — zero-dependency Python SDK for the beatbox sandbox REST API.

Quickstart::

    from beatbox import Client, ExecuteRequest

    client = Client("http://127.0.0.1:7300", token="...")
    result = client.execute(ExecuteRequest.wasm_wat(
        '(module (func (export "run") (param i64) (result i64)'
        ' local.get 0 i64.const 1 i64.add))',
        input={"n": 41},
    ))
    print(result.value)  # 42
"""

from __future__ import annotations

from .client import Client
from .errors import BeatboxApiError, BeatboxError, BeatboxTransportError
from .models import (
    CreateJobResponse,
    EffectiveIsolation,
    EgressRecord,
    ErrorBody,
    ExecuteRequest,
    ExecutionResult,
    ExecutionStatus,
    JobRecord,
    JobStatus,
    Lane,
    Limits,
    Metrics,
    Operation,
    OperationMetadata,
    Policy,
    Source,
    UNSET,
)

__version__ = "0.1.0"

__all__ = [
    "Client",
    "BeatboxError",
    "BeatboxApiError",
    "BeatboxTransportError",
    "ExecuteRequest",
    "Source",
    "Policy",
    "Limits",
    "ExecutionResult",
    "Metrics",
    "EffectiveIsolation",
    "EgressRecord",
    "JobRecord",
    "Operation",
    "OperationMetadata",
    "CreateJobResponse",
    "ErrorBody",
    "ExecutionStatus",
    "JobStatus",
    "Lane",
    "UNSET",
    "__version__",
]
