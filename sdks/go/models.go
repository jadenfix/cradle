package beatbox

import "encoding/json"

// Lane selects the execution engine used by the daemon.
type Lane string

const (
	LaneWasm         Lane = "wasm"
	LanePythonWASI   Lane = "python_wasi"
	LanePythonNative Lane = "python_native"
	LaneJSWasm       Lane = "js_wasm"
	LaneJSNative     Lane = "js_native"
	LaneExec         Lane = "exec"
)

// ExecutionStatus is the terminal status of an execution.
type ExecutionStatus string

const (
	ExecutionStatusOK      ExecutionStatus = "ok"
	ExecutionStatusError   ExecutionStatus = "error"
	ExecutionStatusTimeout ExecutionStatus = "timeout"
	ExecutionStatusOOM     ExecutionStatus = "oom"
	ExecutionStatusKilled  ExecutionStatus = "killed"
	ExecutionStatusDenied  ExecutionStatus = "denied"
)

// JobStatus is the lifecycle status of an asynchronous job.
type JobStatus string

const (
	JobStatusQueued    JobStatus = "queued"
	JobStatusRunning   JobStatus = "running"
	JobStatusSucceeded JobStatus = "succeeded"
	JobStatusFailed    JobStatus = "failed"
	JobStatusCanceled  JobStatus = "canceled"
)

// MountMode is the access mode of a filesystem mount.
type MountMode string

const (
	MountModeRO MountMode = "ro"
	MountModeRW MountMode = "rw"
)

// SecretExpose selects how a secret is exposed to the guest.
type SecretExpose string

const (
	SecretExposeEnv  SecretExpose = "env"
	SecretExposeFile SecretExpose = "file"
)

// SourceKind is the discriminator of the [Source] tagged union.
type SourceKind string

const (
	SourceKindInline          SourceKind = "inline"
	SourceKindWasmFile        SourceKind = "wasm_file"
	SourceKindWasmWat         SourceKind = "wasm_wat"
	SourceKindWasmBytesBase64 SourceKind = "wasm_bytes_base64"
	SourceKindModuleRef       SourceKind = "module_ref"
)

// ExecuteRequest is the body sent to Execute and CreateJob.
//
// Lane and Source are required; the remaining fields are optional and omitted
// from the wire payload when unset. Build one with a constructor such as
// [WasmWatRequest] for the common case, or populate the struct directly.
type ExecuteRequest struct {
	Lane           Lane    `json:"lane"`
	Source         Source  `json:"source"`
	Entrypoint     *string `json:"entrypoint,omitempty"`
	Input          any     `json:"input,omitempty"`
	Stdin          *string `json:"stdin,omitempty"`
	Policy         *Policy `json:"policy,omitempty"`
	IdempotencyKey *string `json:"idempotency_key,omitempty"`
}

// Source is a tagged union (on Kind) describing the code to run. Prefer the
// variant constructors ([SourceInline], [SourceWasmFile], [SourceWasmWat],
// [SourceWasmBytesBase64], [SourceModuleRef]) over building it directly.
type Source struct {
	Kind SourceKind `json:"kind"`
	// Code is set for the "inline" variant.
	Code string `json:"code,omitempty"`
	// Path is set for the "wasm_file" variant.
	Path string `json:"path,omitempty"`
	// Text is set for the "wasm_wat" variant.
	Text string `json:"text,omitempty"`
	// Bytes is the base64-encoded module for the "wasm_bytes_base64" variant.
	Bytes string `json:"bytes,omitempty"`
	// SHA256 is set for the "module_ref" variant.
	SHA256 string `json:"sha256,omitempty"`
}

// Policy is an optional, partial execution policy. Only the fields that are set
// are sent; the daemon merges them onto its defaults.
type Policy struct {
	Limits      *Limits           `json:"limits,omitempty"`
	Determinism *Determinism      `json:"determinism,omitempty"`
	Env         map[string]string `json:"env,omitempty"`
	Fs          *FsPolicy         `json:"fs,omitempty"`
	Net         *NetPolicy        `json:"net,omitempty"`
	Secrets     []Secret          `json:"secrets,omitempty"`
	DoubleJail  *bool             `json:"double_jail,omitempty"`
}

// Limits is a partial set of resource limits. Every field is a pointer so that
// a partial policy only transmits the limits that were explicitly set.
type Limits struct {
	WallMs      *uint64 `json:"wall_ms,omitempty"`
	CPUMs       *uint64 `json:"cpu_ms,omitempty"`
	MemoryBytes *uint64 `json:"memory_bytes,omitempty"`
	DiskBytes   *uint64 `json:"disk_bytes,omitempty"`
	OutputBytes *uint64 `json:"output_bytes,omitempty"`
	Fuel        *uint64 `json:"fuel,omitempty"`
	Pids        *uint32 `json:"pids,omitempty"`
}

// Determinism configures deterministic execution (kind "off" or "seeded").
type Determinism struct {
	Kind    string  `json:"kind"`
	Seed    *uint64 `json:"seed,omitempty"`
	EpochMs *uint64 `json:"epoch_ms,omitempty"`
}

// FsPolicy configures the guest filesystem.
type FsPolicy struct {
	Workspace *string `json:"workspace,omitempty"`
	Mounts    []Mount `json:"mounts,omitempty"`
}

// Mount is a single host->guest filesystem mount.
type Mount struct {
	Host  string    `json:"host"`
	Guest string    `json:"guest"`
	Mode  MountMode `json:"mode"`
}

// NetPolicy configures network egress (kind "deny" or "proxy").
type NetPolicy struct {
	Kind         string   `json:"kind"`
	AllowDomains []string `json:"allow_domains,omitempty"`
	AllowPorts   []int32  `json:"allow_ports,omitempty"`
}

// Secret references a secret to expose to the guest.
type Secret struct {
	Name     string       `json:"name"`
	ValueRef string       `json:"value_ref"`
	Expose   SecretExpose `json:"expose"`
}

// Metrics reports resource usage for an execution. cpu_time_ms, fuel_used and
// peak_memory_bytes are nullable on the wire (the W0 wasm lane, for example,
// does not measure CPU time separately from wall time) and are modeled as
// pointers so that null is representable.
type Metrics struct {
	WallTimeMs      uint64  `json:"wall_time_ms"`
	CPUTimeMs       *uint64 `json:"cpu_time_ms"`
	FuelUsed        *uint64 `json:"fuel_used"`
	PeakMemoryBytes *uint64 `json:"peak_memory_bytes"`
}

// EffectiveIsolation describes the isolation mechanisms actually applied.
type EffectiveIsolation struct {
	OS          string   `json:"os"`
	Mechanisms  []string `json:"mechanisms"`
	Downgrades  []string `json:"downgrades"`
	LandlockABI *int32   `json:"landlock_abi,omitempty"`
}

// EgressRecord is a single observed network egress destination.
type EgressRecord struct {
	Domain string `json:"domain"`
	Port   int32  `json:"port"`
	Bytes  uint64 `json:"bytes"`
}

// ErrorBody is the {code, message} pair carried by API error responses and by
// the error field of a result/job.
type ErrorBody struct {
	Code    string `json:"code"`
	Message string `json:"message"`
}

// errorResponse is the {"error": {...}} envelope returned on non-2xx responses.
type errorResponse struct {
	Error ErrorBody `json:"error"`
}

// ExecutionResult is the synchronous result of Execute (and the result field of
// a finished job). Value holds the raw JSON returned by the guest; decode it
// with encoding/json.
type ExecutionResult struct {
	Status             ExecutionStatus    `json:"status"`
	Value              json.RawMessage    `json:"value"`
	Stdout             string             `json:"stdout"`
	StdoutTruncated    bool               `json:"stdout_truncated"`
	Stderr             string             `json:"stderr"`
	StderrTruncated    bool               `json:"stderr_truncated"`
	Metrics            Metrics            `json:"metrics"`
	Lane               Lane               `json:"lane"`
	Deterministic      bool               `json:"deterministic"`
	InputsDigest       string             `json:"inputs_digest"`
	EngineVersion      string             `json:"engine_version"`
	BeatboxVersion     string             `json:"beatbox_version"`
	EffectiveIsolation EffectiveIsolation `json:"effective_isolation"`
	Egress             []EgressRecord     `json:"egress"`
	Error              *ErrorBody         `json:"error,omitempty"`
	ExitCode           *int32             `json:"exit_code,omitempty"`
}

// JobRecord is the stored state of an asynchronous job.
type JobRecord struct {
	JobID     string           `json:"job_id"`
	Status    JobStatus        `json:"status"`
	Request   ExecuteRequest   `json:"request"`
	Result    *ExecutionResult `json:"result,omitempty"`
	Error     *ErrorBody       `json:"error,omitempty"`
	CreatedAt string           `json:"created_at"`
	UpdatedAt string           `json:"updated_at"`
}

// CreateJobResponse is returned (202) by CreateJob.
type CreateJobResponse struct {
	JobID string `json:"job_id"`
}
