param(
    [Parameter(Mandatory)][string]$Exe,
    [switch]$Parallel,
    [hashtable]$More = @{}
)

$root = Split-Path -Parent $PSScriptRoot
$victory = Join-Path $root 'target\diag\victory.exe'
if (-not (Test-Path -LiteralPath $victory -PathType Leaf)) {
    throw "Build the diagnostics executable first: $victory"
}
$game = (Resolve-Path -LiteralPath $Exe -ErrorAction Stop).Path

$settings = @{
    VICTORY_RELAY = '0'
    VICTORY_LATE_WAIT = '0'
    VICTORY_RELAY_COPY = '0'
    VICTORY_RELAY_NEXT = '2'
    VICTORY_RELAY_NEXT_SNAPSHOTS = '1'
    VICTORY_RELAY_NEXT_MERGE = '0'
    VICTORY_RELAY_NEXT_RESIDENT = '1'
    VICTORY_RELAY_NEXT_READBACK_KB = '16'
    VICTORY_RELAY_NEXT_PACKED_READBACK = '1'
    VICTORY_RELAY_NEXT_SCHEDULER = '1'
    VICTORY_RELAY_NEXT_QUEUE_DEPTH = '8'
    VICTORY_RELAY_NEXT_JOIN_PARTS = '7'
    VICTORY_RELAY_NEXT_DEPENDENCY_BATCHES = '1'
    VICTORY_RELAY_NEXT_CROSS_QUEUE = '1'
    VICTORY_RELAY_NEXT_MERGE_WORKERS = '4'
    VICTORY_RELAY_NEXT_TRACE = '0'
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
foreach ($name in $More.Keys) { $settings[$name] = $More[$name] }

$paths = 'VICTORY_CONFIG', 'VICTORY_GAME_DIR', 'VICTORY_CACHE_DIR', 'VICTORY_DATA_DIR'
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
        -ArgumentList "run `"$game`" --execute" `
        -NoNewWindow -Wait `
        -RedirectStandardOutput "$root\victory.log" `
        -RedirectStandardError "$root\victory-errors.log"
} finally {
    foreach ($name in $before.Keys) {
        [Environment]::SetEnvironmentVariable($name, $before[$name], 'Process')
    }
    Pop-Location
}
