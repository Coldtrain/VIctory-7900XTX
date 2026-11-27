param(
    [Parameter(Mandatory)][string]$Exe,
    [switch]$Parallel,
    [switch]$RNext,
    [switch]$Relay2,
    [switch]$CoolFoxResidency,
    [switch]$CoolFoxThin,
    [ValidateSet('quality','balanced','performance','ultra-performance','720p')][string]$Fsr4K,
    [hashtable]$More = @{}
)

$root = Split-Path -Parent $PSScriptRoot
$victory = Join-Path $root 'target\diag\victory.exe'
if (-not (Test-Path -LiteralPath $victory -PathType Leaf)) {
    throw "Build the diagnostics executable first: $victory"
}
$game = (Resolve-Path -LiteralPath $Exe -ErrorAction Stop).Path

$settings = @{
    # Port of the NVIDIA fork's resolution-only settings; no GPU vendor changes.
    #VICTORY_RENDER_SCALE = '0.20'
    VICTORY_OWN_UPSCALER = '1'
    # Sprint 14a: match title and frame clock at 60 Hz; diagnostics for slow simulation.
    VICTORY_FRAME_INTERVAL_MS = '16.667'
    VICTORY_TITLE_FPS = '60'
    VICTORY_TIMER_LOG = '1'
    VICTORY_RELAY = '0'
    VICTORY_LATE_WAIT = '0'
    VICTORY_RELAY_COPY = '0'
    VICTORY_RELAY_NEXT = '0'
    VICTORY_ASYNC_SUBMIT = '1'
    VICTORY_THIN = '0'
    VICTORY_UPLOAD_AHEAD = '1'
    VICTORY_CARD_STEPS = '0'
    VICTORY_TEXTURE_NO_WAIT = '0'
    VICTORY_D3D12_DRED = 'light'
    VICTORY_D3D12_DEBUG = '0'
    VICTORY_LOG = 'warn,victory_gpu::host=info,victory_gpu::relay_next=info,victory_gpu::queue::behind=info'
    VICTORY_AMBIENT = '0.8'
    VICTORY_SHOW_AFTER = '1'
    VICTORY_SKIP_PIPELINES = 'none'
    # Belobug renderer optimizations; CBV remains off due to artifacts.
    VICTORY_PORT_CONSTANTS = '1'
    VICTORY_PORT_DESCRIPTORS = '1'
    VICTORY_PORT_CBV = '0'
    VICTORY_PORT_CBV_PLACE = '0'
    VICTORY_PORT_CBV_RAW16 = '0'
    VICTORY_PORT_AMD_TEXTURE_BUDGET = '1' # Sprint 12a: adaptive mip and VRAM policy
    VICTORY_PORT_AMD_BATCH_POOL = '1' # Sprint 12b: shared indirect argument buffers
    VICTORY_PORT_AMD_PROBE_FILTER = '1' # Sprint 12c: filter suspect magenta ambient probes
    VICTORY_PORT_GUARD = '1'
    VICTORY_PORT_JOURNAL = '1'
    VICTORY_PORT_TEXTURE_PRELUDE = '1' # Belobug submission-time uploads (Relay 0)
}

if ($RNext) {
    $settings.VICTORY_PORT_TEXTURE_PRELUDE = '0'
    $settings.VICTORY_RELAY_NEXT = '2'
    $settings.VICTORY_RELAY_NEXT_SNAPSHOTS = '1'
    $settings.VICTORY_RELAY_NEXT_MERGE = '0'
    $settings.VICTORY_RELAY_NEXT_RESIDENT = '1'
    $settings.VICTORY_RELAY_NEXT_READBACK_KB = '16'
    $settings.VICTORY_RELAY_NEXT_PACKED_READBACK = '1'
    $settings.VICTORY_RELAY_NEXT_PACKED_MB = '96'
    $settings.VICTORY_RELAY_NEXT_SCHEDULER = '1'
    $settings.VICTORY_RELAY_NEXT_QUEUE_DEPTH = '8'
    $settings.VICTORY_RELAY_NEXT_JOIN_PARTS = '7'
    $settings.VICTORY_RELAY_NEXT_DEPENDENCY_BATCHES = '1'
    $settings.VICTORY_RELAY_NEXT_CROSS_QUEUE = '1'
    $settings.VICTORY_RELAY_NEXT_SPARSE_HASH = '1'
    $settings.VICTORY_RELAY_NEXT_INDEXED_REFRESH = '1'
    $settings.VICTORY_RELAY_NEXT_REFRESH_CHUNK = '8'
    $settings.VICTORY_RELAY_NEXT_TICKET_OVERLAP = '1'
    $settings.VICTORY_RELAY_NEXT_OVERLAP_COMPARE = '1'
    $settings.VICTORY_RELAY_NEXT_INDEPENDENT_FLIGHTS = '1'
    $settings.VICTORY_RELAY_NEXT_ORDERED_SIGNALS = '1'
    $settings.VICTORY_RELAY_NEXT_PERSISTENT_FLIGHTS = '1'
    $settings.VICTORY_RELAY_NEXT_MAX_FLIGHTS = '4'
    $settings.VICTORY_RELAY_NEXT_MERGE_WORKERS = '4'
    $settings.VICTORY_RELAY_NEXT_TRACE = '0'
}

if ($Parallel) {
    $settings.VICTORY_ASYNC_SUBMIT = '1'
    $settings.VICTORY_THIN = '1'
    $settings.VICTORY_THIN_THREADS = '8'
    $settings.VICTORY_UPLOAD_AHEAD = '1'
    $settings.VICTORY_UPLOAD_WORKERS = '8'
    $settings.VICTORY_INLINE_READBACK = '1'
    $settings.VICTORY_LOG = 'warn,victory_gpu::host=info,victory_gpu::relay_next=info,victory_gpu::list::thin=info,victory_gpu::queue::behind=info,victory_gpu::texture::ahead=info'
}
# Configuration experiments from codec-d/cool_fox Play-VIctory.cmd.
# Source: https://github.com/codec-d/cool_fox/blob/main/Play-VIctory.cmd
# Relay 0 is the baseline. Relay Next is opt-in via -RNext.
# Preserve the console profile and visibility rules.
# Test one switch at a time; neither is enabled by default.
if ($CoolFoxResidency) {
    $settings.VICTORY_CARD_BUFFERS_MIN_KB = '64'
    # Keep the existing adapter-specific allocation budget and residency guards.
}
if ($CoolFoxThin) {
    $settings.VICTORY_THIN = '1'
    $settings.VICTORY_THIN_THREADS = '8'
    # Unlike -Parallel, do not enable inline readback or change upload workers.
    $settings.VICTORY_LOG = 'warn,victory_gpu::host=info,victory_gpu::relay_next=info,victory_gpu::list::thin=info,victory_gpu::queue::behind=info'
}
if ($Relay2) {
    if ($RNext) { throw 'Choose -Relay2 or -RNext, not both.' }
    $settings.VICTORY_RELAY = '2'
    $settings.VICTORY_LATE_WAIT = '3'
    $settings.VICTORY_RELAY_NEXT = '0'
    $settings.VICTORY_RELAY_COPY = '0'
    $settings.VICTORY_KEEP_SNAPSHOTS = '0'
    $settings.VICTORY_LATE_WAIT_LISTS = '32'
    $settings.VICTORY_PORT_TEXTURE_PRELUDE = '0'
    # First isolate the legacy landing thread. -Parallel explicitly opts
    # into thin recording; without it do not add async submission.
    if (-not $Parallel -and -not $CoolFoxThin) {
        $settings.VICTORY_ASYNC_SUBMIT = '0'
        $settings.VICTORY_THIN = '0'
        $settings.VICTORY_INLINE_READBACK = '0'
    }
}
$settings.VICTORY_LOG = 'warn,victory_gpu=info,victory_loader=info'
if ($Fsr4K) { $settings.VICTORY_FSR_4K = $Fsr4K.ToLowerInvariant() }
foreach ($name in $More.Keys) { $settings[$name] = $More[$name] }
# JoshuuDev legacy relay path: do not combine with Relay 0 upload prelude.
if ($settings.VICTORY_RELAY -in @('1','2') -and $settings.VICTORY_RELAY_REFERENCE -ne '0') {
    $settings.VICTORY_RELAY_REFERENCE = '1'
    $settings.VICTORY_RELAY_NEXT = '0'
    $settings.VICTORY_PORT_TEXTURE_PRELUDE = '0'
    $settings.VICTORY_CARD_STEPS = '0'
    $settings.VICTORY_RELAY_COPY = '0'
    $settings.VICTORY_ASYNC_SUBMIT = '0'
    $settings.VICTORY_INLINE_READBACK = '0'
    $settings.VICTORY_KEEP_SNAPSHOTS = '0'
    if (-not $More.ContainsKey('VICTORY_LATE_WAIT')) { $settings.VICTORY_LATE_WAIT = '2' }
    if (-not $More.ContainsKey('VICTORY_LATE_WAIT_WRITES')) { $settings.VICTORY_LATE_WAIT_WRITES = '0' }
    if (-not $More.ContainsKey('VICTORY_LATE_WAIT_LISTS')) { $settings.VICTORY_LATE_WAIT_LISTS = '128' }
    # Descriptor publication and its journal deliberately remain untouched.
}
$console = 'series-s'
if ($settings.VICTORY_FSR_4K -and $settings.VICTORY_FSR_4K -ne '0') {
    # Do not compound the separate experimental FSR path with new timing.
    if (-not $More.ContainsKey('VICTORY_FRAME_INTERVAL_MS')) { $settings.VICTORY_FRAME_INTERVAL_MS = '' }
    if ($Relay2) { throw 'Test Relay 2 without FSR first.' }
    if ($settings.VICTORY_SIM_MAX_MS -and $settings.VICTORY_SIM_MAX_MS -ne '0') { throw 'Test simulation-step override without FSR first.' }
    $console = 'series-x'
    $settings.VICTORY_OWN_UPSCALER = '0'
    $settings.VICTORY_RENDER_SCALE = '0'
    $settings.VICTORY_PORT_TEXTURE_PRELUDE = '0'
    $settings.VICTORY_RELAY = '0'
    $settings.VICTORY_LATE_WAIT = '0'
    $settings.VICTORY_RELAY_NEXT = '0'
    $settings.VICTORY_ASYNC_SUBMIT = '0'
    $settings.VICTORY_THIN = '0'
    $settings.VICTORY_LOG = 'warn,victory_gpu=info,victory_loader=info,victory_hle::modules::fidelityfx=info'
}

$paths = 'VICTORY_CONFIG', 'VICTORY_GAME_DIR', 'VICTORY_CACHE_DIR', 'VICTORY_DATA_DIR', 'VICTORY_FIDELITYFX_DIR'
$reset = @(Get-ChildItem Env:VICTORY_* | Where-Object Name -NotIn $paths)
$before = @{}
Push-Location -LiteralPath $root
try {
    foreach ($name in (@($reset | Select-Object -ExpandProperty Name) + @($settings.Keys))) {
        if ($before.ContainsKey($name)) { continue }
        $before[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
        [Environment]::SetEnvironmentVariable($name, $settings[$name], 'Process')
    }
    Start-Process -FilePath $victory `
        -ArgumentList "run `"$game`" --execute --console $console" `
        -NoNewWindow -Wait `
        -RedirectStandardOutput "$root\victory.log" `
        -RedirectStandardError "$root\victory-errors.log"
} finally {
    foreach ($name in $before.Keys) {
        [Environment]::SetEnvironmentVariable($name, $before[$name], 'Process')
    }
    Pop-Location
}
