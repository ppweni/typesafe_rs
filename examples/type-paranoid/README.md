# TypeParanoid

**Trust types. Question traffic.**

A standalone binary example of the TypeSafe Rust SDK. It embeds packet capture and
parsing in Rust, summarizes TCP/UDP traffic involving the current host, and flags
unusual outbound transfers. TypeSafe can assess selected candidates when enabled.
There is no demo command; synthetic traffic scenarios live exclusively in tests.

## Run

From the repository root:

```sh
cargo build -p type-paranoid
cargo run -p type-paranoid -- interfaces
```

Select an active Ethernet-compatible interface from that list. For example, on
macOS, with permission to access `/dev/bpf*`:

```sh
sudo ./target/debug/type-paranoid monitor --interface en0 --duration-secs 60
```

For a live terminal dashboard:

```sh
sudo ./target/debug/type-paranoid monitor --interface en1 --pretty
```

The Ratatui dashboard shows outbound and inbound rate graphs for the last two
minutes, total transferred bytes, capture counters, recent candidates, and TypeSafe
results when `--assess` is also enabled. Rates are sampled approximately once per
second using actual elapsed time; the display refreshes four times per second.
Both graphs use the same vertical scale. Assessment windows still default to ten
seconds, independent of the display refresh rate.

Press `q`, Escape, or Ctrl-C to stop. The terminal is restored before the final
local summary is printed, including on errors. At least 72 columns by 24 rows are
needed for graphs; smaller terminals show a compact status view. `--pretty`
requires interactive stdin/stdout and cannot be combined with `--json`.

The interface name is explicit because guessing can select the wrong network,
especially on machines with virtual interfaces. On Linux, use the appropriate
interface (often `eth0` in a container) with `CAP_NET_RAW` available. Root is a
simple local setup; the program itself does not modify capture permissions.

Capture uses `pnet_datalink`'s native BPF/AF_PACKET backends. Parsing uses
`etherparse`. No libpcap, external capture tools, or subprocesses are required.
The capture backend uses OS bindings through `libc`.

Local rules run by default. To send candidate metadata to TypeSafe, set
`TYPESAFE_API_KEY` and add `--assess`; `TYPESAFE_BASE_URL` and
`TYPESAFE_DEFAULT_MODEL` work as in the SDK. If using sudo, preserve the required
environment variables in accordance with your local sudo configuration. Do not
put the API key on the command line.

On macOS, `cargo run` as an ordinary user may fail with `No such file or directory`
even when `/dev/bpf0` exists. The capture backend tries numbered BPF devices and
returns only the last error, which can hide earlier permission failures. Check
`ls -l /dev/bpf0`: devices owned by root with mode `crw-------` require elevated
access. Build as your normal user, then use sudo for the compiled binary as shown
above; there is no need to run Cargo itself as root.

```sh
./target/debug/type-paranoid monitor --interface eth0 --assess --json
```

Use `--help` or `monitor --help` for the complete CLI. Useful options:

| Option | Default | Meaning |
|---|---|---|
| `--window-secs` | 10 | Observation window length |
| `--duration-secs` | Until Ctrl-C | Stop after a bounded duration |
| `--upload-bytes` | 10485760 | Large outbound transfer threshold per window |
| `--min-upload-bytes` | 262144 | Minimum size for novelty and rate-spike rules |
| `--expected-peer` | None | Suppress candidates for an expected IP:port; repeatable |
| `--assess` | Off | Enable TypeSafe API calls |
| `--json` | Off | Newline-delimited JSON instead of readable output |
| `--pretty` | Off | Live traffic graphs and assessment dashboard |

An expected peer is an explicit exception, for example a known backup receiver:

```sh
./target/debug/type-paranoid monitor --interface eth0 \
  --expected-peer 192.0.2.20:443 --window-secs 10
```

## What it detects

The collector reads both directions, classifies them using local interface
addresses, and groups observations by remote IP, remote port, and TCP/UDP protocol.
All local processes contacting the same peer are aggregated. Local heuristics flag:

- Outbound payload volume reaching the large-transfer threshold.
- A peer absent from retained history receiving at least the minimum upload size,
  with outbound bytes at least ten times inbound bytes.
- Outbound bytes per second reaching four times the historical mean, after at
  least three previous windows, and reaching the minimum upload size.

History is in memory, retains up to six previous windows per peer, and expires
inactive peers. “New” means absent from retained observations, not newly registered
or malicious. Expected peers are omitted from candidate selection. The first
window has no baseline; a large legitimate upload can be flagged.

TypeSafe receives candidate IP:ports, protocol, counters, observation duration,
baseline statistics, local signals, and the fact that process identity is unknown.
It returns `routine`, `unusual`, or `insufficient_evidence`, plus a review-priority
score from 0 to 3. These are model assessments, not calibrated probabilities of
data theft. API failures leave local findings available and emit a distinct error.

No packet payload, file contents, commands, or API keys are included in assessment
state. TLS is not decrypted. Observed payload bytes include retransmissions and
encryption overhead; they are not exact unique application-byte counts.

## Implementation and limits

`main.rs` only parses and dispatches. `cli.rs` defines arguments; `monitor.rs`
coordinates capture, windows, and inference; `capture.rs` owns the native capture
thread; `packet.rs` parses borrowed frames; `engine.rs` owns aggregation and history;
`api.rs` configures endpoint exclusion; `assess.rs` builds TypeSafe questions;
`presentation.rs` routes events to the selected output mode; `output.rs` renders
plain/JSON findings; `dashboard/` owns terminal lifecycle, bounded live state, and
Ratatui rendering. Assessment tasks return results to the main loop so they never
print over the dashboard.

- The capture thread copies only packet metadata into a bounded 8192-event channel.
  Full queues drop events and increment a visible counter. Kernel drops are unknown.
- Current traffic and history each have a 4096-peer cap. Overflow and omitted
  candidates are reported. The eight largest candidates per window are retained.
- At most two assessment requests run concurrently. Busy slots cause assessments
  to be skipped explicitly; local analysis continues. Repeated qualifying windows
  can produce repeated alerts/API calls; there is no persistent incident tracking.
- In assessment mode, endpoint addresses are resolved once, pinned in the HTTP
  client, and excluded in both directions. Other traffic to those exact IP:ports is
  also excluded. The example disables HTTP proxy discovery to keep this exclusion
  consistent. Restart to refresh DNS or changed local interface addresses.
- Ctrl-C or the duration limit stops capture, prints the final local window, and
  cancels pending assessments. The final partial window is local-only.
- Capture covers one Ethernet-compatible interface, including IPv4/IPv6 and VLAN
  frames. Loopback and point-to-point/VPN interfaces are rejected. Fragmented IP
  traffic and malformed packets are skipped and counted; other protocols are ignored.
- **Process attribution is not implemented.** Findings report `process: null` /
  `process=unknown`. This can flag an anomalous process's transfers, but cannot name
  or terminate the process. It also does not block traffic, extract hostnames, or
  inspect encrypted content. Slow or distributed transfers can evade these rules.

## Test

```sh
cargo test -p type-paranoid
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

Tests build synthetic packets and exercise normal downloads, expected backups,
new uploads, rate spikes, bounded history, exclusions, and malformed packets. A
local HTTP fixture verifies the actual SDK assessment request and response. No
capture privileges, API key, or paid inference is needed; HTTP tests need localhost
socket access.

Docker capture testing is a subsequent step. Run the monitor and a traffic
generator in the same container/network namespace, with the receiver in another
container. On Docker Desktop this observes the container's traffic, not the Mac's
physical interfaces. The current example contains no Docker deployment or test
traffic generator.
