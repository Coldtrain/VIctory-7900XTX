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
    VICTORY_RELAY = '2'
    VICTORY_RELAY_COPY = '0'
    VICTORY_RELAY_TRACE = '1'
    VICTORY_ASYNC_SUBMIT = '1'
    VICTORY_THIN = '0'
    VICTORY_UPLOAD_AHEAD = '1'
    VICTORY_CARD_STEPS = '0'
    VICTORY_TEXTURE_NO_WAIT = '0'
    VICTORY_D3D12_DRED = 'light'
    VICTORY_D3D12_DEBUG = '1'
    VICTORY_LOG = 'warn,victory_gpu::host=info'
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
    $settings.VICTORY_LOG = 'warn,victory_gpu::host=info,victory_gpu::list::thin=info,victory_gpu::queue::behind=info,victory_gpu::texture::ahead=info'
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
    & $victory run $game --execute
} finally {
    foreach ($name in $before.Keys) {
        [Environment]::SetEnvironmentVariable($name, $before[$name], 'Process')
    }
    Pop-Location
}
