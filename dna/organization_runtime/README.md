# Prepared Organization runtime reads

`OrganizationRuntime { root }.current(application_id, request_id, attempt_id)`
returns a `dna::OrganizationRuntimeAssociation`. Host and an explicitly composed
API use this one implementation. It starts no process and acquires no lease.

An available result requires the exact admitted source request, native candidate,
application and restart handoff; the captured host attempt and launch; the same
owning Host process and authoritative body lease; and the same live child,
executable, retained topology and opaque Iris process key. Missing or mismatched
evidence returns the default unavailable association. Historical launch and
health facts do not establish current liveness.

The reader must be co-located with the owning Host's filesystem and process
namespace. Caller authorization and public metadata filtering remain the API's
responsibility. The public API defaults to an unavailable provider; its configured
source evidence head may inject this concrete reader after checking its separate
source read/recovery authority.

`HostAuthority` preserves existing authority routing: a service lease after
Record routing adoption; the remote Record's lease when configured; otherwise the
local Record cell. A remote read refreshes its local tracking cell (or removes an
absent tracking cell). These mirror updates are existing read behavior, not lease
acquisition, renewal or release. Host retains all lease writes and orchestration.
