"""Typed models mirroring the ``components.schemas`` in ``sdks/openapi.json``.

Wire field names are snake_case and are preserved exactly on serialization.
Deserialization ignores unknown/extra fields so newer daemons stay compatible
with older SDKs (forward-compat).
"""

from __future__ import annotations

from dataclasses import dataclass, field
from enum import Enum
from typing import Any, Dict, List, Mapping, Optional


# ---------------------------------------------------------------------------
# Sentinel for "field was not provided" vs. an explicit ``None`` JSON value.
# ---------------------------------------------------------------------------
class _Unset:
    _instance: Optional["_Unset"] = None

    def __new__(cls) -> "_Unset":
        if cls._instance is None:
            cls._instance = super().__new__(cls)
        return cls._instance

    def __repr__(self) -> str:  # pragma: no cover - cosmetic
        return "UNSET"

    def __bool__(self) -> bool:
        return False


UNSET = _Unset()


# ---------------------------------------------------------------------------
# Enums (string-valued, forward-compatible)
# ---------------------------------------------------------------------------
class ExecutionStatus(str, Enum):
    OK = "ok"
    ERROR = "error"
    TIMEOUT = "timeout"
    OOM = "oom"
    KILLED = "killed"
    DENIED = "denied"


class JobStatus(str, Enum):
    QUEUED = "queued"
    RUNNING = "running"
    SUCCEEDED = "succeeded"
    FAILED = "failed"
    CANCELED = "canceled"


class Lane(str, Enum):
    WASM = "wasm"
    PYTHON_WASI = "python_wasi"
    PYTHON_NATIVE = "python_native"
    JS_WASM = "js_wasm"
    JS_NATIVE = "js_native"
    EXEC = "exec"


def _enum_or_raw(enum_cls, value):
    """Parse an enum value, keeping the raw string if it is unknown."""
    if value is None:
        return None
    try:
        return enum_cls(value)
    except ValueError:
        return value


# ---------------------------------------------------------------------------
# Source (tagged union on ``kind``)
# ---------------------------------------------------------------------------
@dataclass
class Source:
    """A program source. Construct via the per-variant classmethods."""

    kind: str
    code: Optional[str] = None
    path: Optional[str] = None
    text: Optional[str] = None
    bytes: Optional[str] = None
    sha256: Optional[str] = None

    @classmethod
    def inline(cls, code: str) -> "Source":
        return cls(kind="inline", code=code)

    @classmethod
    def wasm_file(cls, path: str) -> "Source":
        return cls(kind="wasm_file", path=path)

    @classmethod
    def wasm_wat(cls, text: str) -> "Source":
        return cls(kind="wasm_wat", text=text)

    @classmethod
    def wasm_bytes_base64(cls, bytes: str) -> "Source":  # noqa: A002 - wire name
        return cls(kind="wasm_bytes_base64", bytes=bytes)

    @classmethod
    def module_ref(cls, sha256: str) -> "Source":
        return cls(kind="module_ref", sha256=sha256)

    def to_dict(self) -> Dict[str, Any]:
        out: Dict[str, Any] = {"kind": self.kind}
        for name in ("code", "path", "text", "bytes", "sha256"):
            value = getattr(self, name)
            if value is not None:
                out[name] = value
        return out

    @classmethod
    def from_dict(cls, data: Mapping[str, Any]) -> "Source":
        return cls(
            kind=data.get("kind", ""),
            code=data.get("code"),
            path=data.get("path"),
            text=data.get("text"),
            bytes=data.get("bytes"),
            sha256=data.get("sha256"),
        )


# ---------------------------------------------------------------------------
# Policy / Limits (partial — only provided fields are serialized)
# ---------------------------------------------------------------------------
@dataclass
class Limits:
    """Partial resource limits merged onto the daemon defaults."""

    cpu_ms: Optional[int] = None
    disk_bytes: Optional[int] = None
    fuel: Optional[int] = None
    memory_bytes: Optional[int] = None
    output_bytes: Optional[int] = None
    pids: Optional[int] = None
    wall_ms: Optional[int] = None

    def to_dict(self) -> Dict[str, Any]:
        out: Dict[str, Any] = {}
        for name in (
            "cpu_ms",
            "disk_bytes",
            "fuel",
            "memory_bytes",
            "output_bytes",
            "pids",
            "wall_ms",
        ):
            value = getattr(self, name)
            if value is not None:
                out[name] = value
        return out

    @classmethod
    def from_dict(cls, data: Mapping[str, Any]) -> "Limits":
        return cls(
            cpu_ms=data.get("cpu_ms"),
            disk_bytes=data.get("disk_bytes"),
            fuel=data.get("fuel"),
            memory_bytes=data.get("memory_bytes"),
            output_bytes=data.get("output_bytes"),
            pids=data.get("pids"),
            wall_ms=data.get("wall_ms"),
        )


@dataclass
class Policy:
    """Partial execution policy. Only provided fields are sent.

    The nested ``determinism``, ``net``, ``fs`` and ``secrets`` shapes are
    passed through as plain JSON so the SDK stays small; ``limits`` is typed.
    """

    limits: Optional[Limits] = None
    env: Optional[Dict[str, str]] = None
    double_jail: Optional[bool] = None
    determinism: Optional[Any] = None
    net: Optional[Any] = None
    fs: Optional[Any] = None
    secrets: Optional[List[Any]] = None

    def to_dict(self) -> Dict[str, Any]:
        out: Dict[str, Any] = {}
        if self.limits is not None:
            out["limits"] = self.limits.to_dict()
        if self.env is not None:
            out["env"] = self.env
        if self.double_jail is not None:
            out["double_jail"] = self.double_jail
        if self.determinism is not None:
            out["determinism"] = self.determinism
        if self.net is not None:
            out["net"] = self.net
        if self.fs is not None:
            out["fs"] = self.fs
        if self.secrets is not None:
            out["secrets"] = self.secrets
        return out

    @classmethod
    def from_dict(cls, data: Mapping[str, Any]) -> "Policy":
        limits = data.get("limits")
        return cls(
            limits=Limits.from_dict(limits) if isinstance(limits, Mapping) else None,
            env=data.get("env"),
            double_jail=data.get("double_jail"),
            determinism=data.get("determinism"),
            net=data.get("net"),
            fs=data.get("fs"),
            secrets=data.get("secrets"),
        )


# ---------------------------------------------------------------------------
# ExecuteRequest
# ---------------------------------------------------------------------------
@dataclass
class ExecuteRequest:
    """A request to execute a program synchronously or as a job."""

    lane: Lane
    source: Source
    entrypoint: Optional[str] = None
    input: Any = UNSET
    stdin: Optional[str] = None
    policy: Optional[Policy] = None
    idempotency_key: Optional[str] = None

    @classmethod
    def wasm_wat(
        cls,
        text: str,
        *,
        lane: Lane = Lane.WASM,
        entrypoint: Optional[str] = None,
        input: Any = UNSET,
        stdin: Optional[str] = None,
        policy: Optional[Policy] = None,
        idempotency_key: Optional[str] = None,
    ) -> "ExecuteRequest":
        return cls(
            lane=lane,
            source=Source.wasm_wat(text),
            entrypoint=entrypoint,
            input=input,
            stdin=stdin,
            policy=policy,
            idempotency_key=idempotency_key,
        )

    @classmethod
    def wasm_bytes_base64(
        cls,
        bytes: str,  # noqa: A002 - wire name
        *,
        lane: Lane = Lane.WASM,
        entrypoint: Optional[str] = None,
        input: Any = UNSET,
        stdin: Optional[str] = None,
        policy: Optional[Policy] = None,
        idempotency_key: Optional[str] = None,
    ) -> "ExecuteRequest":
        return cls(
            lane=lane,
            source=Source.wasm_bytes_base64(bytes),
            entrypoint=entrypoint,
            input=input,
            stdin=stdin,
            policy=policy,
            idempotency_key=idempotency_key,
        )

    def to_dict(self) -> Dict[str, Any]:
        lane = self.lane.value if isinstance(self.lane, Lane) else self.lane
        out: Dict[str, Any] = {"lane": lane, "source": self.source.to_dict()}
        if self.entrypoint is not None:
            out["entrypoint"] = self.entrypoint
        if self.input is not UNSET:
            out["input"] = self.input
        if self.stdin is not None:
            out["stdin"] = self.stdin
        if self.policy is not None:
            out["policy"] = self.policy.to_dict()
        if self.idempotency_key is not None:
            out["idempotency_key"] = self.idempotency_key
        return out

    @classmethod
    def from_dict(cls, data: Mapping[str, Any]) -> "ExecuteRequest":
        policy = data.get("policy")
        return cls(
            lane=_enum_or_raw(Lane, data.get("lane")),
            source=Source.from_dict(data.get("source", {})),
            entrypoint=data.get("entrypoint"),
            input=data["input"] if "input" in data else UNSET,
            stdin=data.get("stdin"),
            policy=Policy.from_dict(policy) if isinstance(policy, Mapping) else None,
            idempotency_key=data.get("idempotency_key"),
        )


# ---------------------------------------------------------------------------
# Response models
# ---------------------------------------------------------------------------
@dataclass
class ErrorBody:
    code: str
    message: str
    status: int = 0
    request_id: str = ""
    retryable: bool = False
    details: list[Mapping[str, Any]] = field(default_factory=list)

    def to_dict(self) -> Dict[str, Any]:
        return {
            "code": self.code,
            "message": self.message,
            "status": self.status,
            "request_id": self.request_id,
            "retryable": self.retryable,
            "details": list(self.details),
        }

    @classmethod
    def from_dict(cls, data: Mapping[str, Any]) -> "ErrorBody":
        return cls(
            code=data.get("code", ""),
            message=data.get("message", ""),
            status=data.get("status", 0),
            request_id=data.get("request_id", ""),
            retryable=data.get("retryable", False),
            details=list(data.get("details", [])),
        )


@dataclass
class Metrics:
    wall_time_ms: int
    cpu_time_ms: Optional[int] = None
    fuel_used: Optional[int] = None
    peak_memory_bytes: Optional[int] = None

    def to_dict(self) -> Dict[str, Any]:
        out: Dict[str, Any] = {"wall_time_ms": self.wall_time_ms}
        for name in ("cpu_time_ms", "fuel_used", "peak_memory_bytes"):
            out[name] = getattr(self, name)
        return out

    @classmethod
    def from_dict(cls, data: Mapping[str, Any]) -> "Metrics":
        return cls(
            wall_time_ms=data.get("wall_time_ms", 0),
            cpu_time_ms=data.get("cpu_time_ms"),
            fuel_used=data.get("fuel_used"),
            peak_memory_bytes=data.get("peak_memory_bytes"),
        )


@dataclass
class EffectiveIsolation:
    os: str = ""
    mechanisms: List[str] = field(default_factory=list)
    downgrades: List[str] = field(default_factory=list)
    landlock_abi: Optional[int] = None

    def to_dict(self) -> Dict[str, Any]:
        return {
            "os": self.os,
            "mechanisms": list(self.mechanisms),
            "downgrades": list(self.downgrades),
            "landlock_abi": self.landlock_abi,
        }

    @classmethod
    def from_dict(cls, data: Mapping[str, Any]) -> "EffectiveIsolation":
        return cls(
            os=data.get("os", ""),
            mechanisms=list(data.get("mechanisms", [])),
            downgrades=list(data.get("downgrades", [])),
            landlock_abi=data.get("landlock_abi"),
        )


@dataclass
class EgressRecord:
    domain: str
    port: int
    bytes: int

    def to_dict(self) -> Dict[str, Any]:
        return {"domain": self.domain, "port": self.port, "bytes": self.bytes}

    @classmethod
    def from_dict(cls, data: Mapping[str, Any]) -> "EgressRecord":
        return cls(
            domain=data.get("domain", ""),
            port=data.get("port", 0),
            bytes=data.get("bytes", 0),
        )


@dataclass
class ExecutionResult:
    status: ExecutionStatus
    value: Any
    stdout: str
    stdout_truncated: bool
    stderr: str
    stderr_truncated: bool
    metrics: Metrics
    lane: Lane
    deterministic: bool
    inputs_digest: str
    engine_version: str
    beatbox_version: str
    effective_isolation: EffectiveIsolation
    egress: List[EgressRecord] = field(default_factory=list)
    error: Optional[ErrorBody] = None
    exit_code: Optional[int] = None

    def to_dict(self) -> Dict[str, Any]:
        status = self.status.value if isinstance(self.status, ExecutionStatus) else self.status
        lane = self.lane.value if isinstance(self.lane, Lane) else self.lane
        return {
            "status": status,
            "value": self.value,
            "stdout": self.stdout,
            "stdout_truncated": self.stdout_truncated,
            "stderr": self.stderr,
            "stderr_truncated": self.stderr_truncated,
            "metrics": self.metrics.to_dict(),
            "lane": lane,
            "deterministic": self.deterministic,
            "inputs_digest": self.inputs_digest,
            "engine_version": self.engine_version,
            "beatbox_version": self.beatbox_version,
            "effective_isolation": self.effective_isolation.to_dict(),
            "egress": [e.to_dict() for e in self.egress],
            "error": self.error.to_dict() if self.error is not None else None,
            "exit_code": self.exit_code,
        }

    @classmethod
    def from_dict(cls, data: Mapping[str, Any]) -> "ExecutionResult":
        err = data.get("error")
        return cls(
            status=_enum_or_raw(ExecutionStatus, data.get("status")),
            value=data.get("value"),
            stdout=data.get("stdout", ""),
            stdout_truncated=data.get("stdout_truncated", False),
            stderr=data.get("stderr", ""),
            stderr_truncated=data.get("stderr_truncated", False),
            metrics=Metrics.from_dict(data.get("metrics", {})),
            lane=_enum_or_raw(Lane, data.get("lane")),
            deterministic=data.get("deterministic", False),
            inputs_digest=data.get("inputs_digest", ""),
            engine_version=data.get("engine_version", ""),
            beatbox_version=data.get("beatbox_version", ""),
            effective_isolation=EffectiveIsolation.from_dict(
                data.get("effective_isolation", {})
            ),
            egress=[EgressRecord.from_dict(e) for e in data.get("egress", [])],
            error=ErrorBody.from_dict(err) if isinstance(err, Mapping) else None,
            exit_code=data.get("exit_code"),
        )


@dataclass
class CreateJobResponse:
    """Legacy pre-Operation create-job response shape."""

    job_id: str

    def to_dict(self) -> Dict[str, Any]:
        return {"job_id": self.job_id}

    @classmethod
    def from_dict(cls, data: Mapping[str, Any]) -> "CreateJobResponse":
        return cls(job_id=data.get("job_id", ""))


@dataclass
class OperationMetadata:
    target_resource: str = ""
    create_time: str = ""
    current_stage: str = ""
    progress_ratio: float = 0.0

    @classmethod
    def from_dict(cls, data: Mapping[str, Any]) -> "OperationMetadata":
        return cls(
            target_resource=data.get("target_resource", ""),
            create_time=data.get("create_time", ""),
            current_stage=data.get("current_stage", ""),
            progress_ratio=float(data.get("progress_ratio", 0.0)),
        )


@dataclass
class Operation:
    name: str
    done: bool
    metadata: Optional[OperationMetadata] = None
    response: Any = None
    error: Optional[ErrorBody] = None

    @classmethod
    def from_dict(cls, data: Mapping[str, Any]) -> "Operation":
        metadata = data.get("metadata")
        err = data.get("error")
        return cls(
            name=data.get("name", ""),
            done=bool(data.get("done", False)),
            metadata=OperationMetadata.from_dict(metadata) if isinstance(metadata, Mapping) else None,
            response=data.get("response"),
            error=ErrorBody.from_dict(err) if isinstance(err, Mapping) else None,
        )


@dataclass
class JobRecord:
    job_id: str
    status: JobStatus
    request: ExecuteRequest
    created_at: str
    updated_at: str
    result: Optional[ExecutionResult] = None
    error: Optional[ErrorBody] = None

    def to_dict(self) -> Dict[str, Any]:
        status = self.status.value if isinstance(self.status, JobStatus) else self.status
        return {
            "job_id": self.job_id,
            "status": status,
            "request": self.request.to_dict(),
            "created_at": self.created_at,
            "updated_at": self.updated_at,
            "result": self.result.to_dict() if self.result is not None else None,
            "error": self.error.to_dict() if self.error is not None else None,
        }

    @classmethod
    def from_dict(cls, data: Mapping[str, Any]) -> "JobRecord":
        result = data.get("result")
        err = data.get("error")
        return cls(
            job_id=data.get("job_id", ""),
            status=_enum_or_raw(JobStatus, data.get("status")),
            request=ExecuteRequest.from_dict(data.get("request", {})),
            created_at=data.get("created_at", ""),
            updated_at=data.get("updated_at", ""),
            result=ExecutionResult.from_dict(result) if isinstance(result, Mapping) else None,
            error=ErrorBody.from_dict(err) if isinstance(err, Mapping) else None,
        )
