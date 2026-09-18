# bkmpw machine protocol

`bkmpw` protocol version 1 provides structured output for desktop applications,
CI jobs, and other process-based integrations. Human-readable output remains the
default.

## Discovering the protocol

```text
bkmpw protocol-version
bkmpw protocol-version --json
```

The plain command prints the integer protocol version. The JSON form also
reports the CLI version and supported machine-output formats.

Consumers must check `protocolVersion`. CLI and protocol versions are separate:
a CLI release may add fields without changing the protocol version.

## Output formats

`--json` may appear before or after the command. It writes exactly one JSON
object followed by a newline to stdout:

```json
{"protocolVersion":1,"command":"inspect","ok":true,"data":{}}
```

`--json-lines` (or `--jsonl`) writes one JSON object per line and flushes every
event. Event order is identified by the monotonically increasing `sequence`
field:

```json
{"protocolVersion":1,"command":"refresh","sequence":1,"event":"started"}
{"protocolVersion":1,"command":"refresh","sequence":2,"event":"progress","phase":"refreshing","message":"rebuilding pack index"}
{"protocolVersion":1,"command":"refresh","sequence":3,"event":"completed","ok":true,"data":{}}
```

The terminal event is either `completed` or `failed`. A process consumer should
read both stdout and stderr concurrently to avoid pipe backpressure. Stdout is
reserved for protocol records; stderr may contain human-readable download and
diagnostic progress from lower-level operations.

## Errors and exit status

Failures preserve the normal CLI exit status and return a structured error:

```json
{
  "protocolVersion": 1,
  "command": "scan",
  "ok": false,
  "error": {
    "code": "IO_ERROR",
    "message": "pack root does not exist"
  }
}
```

Error codes in protocol version 1 are:

- `INVALID_ARGUMENT`
- `IO_ERROR`
- `COMMAND_FAILED`

`check` is special: its `data` is returned even when validation fails. In that
case `data.valid` is false, `ok` is false, and the process exits with status 2.

## Supported commands in protocol version 1

- `version`
- `protocol-version`
- `inspect`
- `check`
- `scan`
- `list`
- `refresh`
- `init`
- `check-updates`
- `update`
- `download-files`
- `install-files`
- `install-files-headless`
- `install-files-retry`
- `install-local`
- `sync`
- `export-client`
- `export-server`
- `export-server-installer`
- `export-curseforge`
- `prepare-server`

Using machine output with another command returns `INVALID_ARGUMENT`. This is
intentional: integrations must never receive human-readable text mixed into
their stdout JSON stream.

## Compatibility rules

Protocol version 1 follows these compatibility rules:

- Existing fields keep their meaning and type.
- New optional fields and event phases may be added without a version bump.
- Consumers must ignore unknown object fields and progress phases.
- Removing a field, changing a field type, or changing command semantics
  requires a protocol-version increment.
- Paths are encoded as platform-native display strings. Consumers must not
  assume `/` separators on Windows.
- JSON object key order is not significant.

For a Tauri or other desktop shell, invoke the executable directly with an
argument array. Do not construct a shell command string.

### Update checks and selection

`check-updates <pack-root> [--all|<name>...]` returns `available` entries with
`path`, `name`, `before`, and `after`, plus `skipped` entries with `path` and `reason`.
Reasons include `update_pinned`, `update_no_provider`, and `update_unchanged`.
Checks do not write pack files; both JSON and JSON Lines are supported.
`update <pack-root> <name>...` accepts multiple names or metadata paths. Its result
adds `targets` with the requested operands (empty for `--all`); repeated names/aliases
resolve to one file. Legacy `target` is the single name/all flag, or `null` for a batch.
Use `--` before names beginning with `-`; put command options before this delimiter.
