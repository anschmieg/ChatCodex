# ChatCodex Deployment Script (Windows)
#
# Usage:
#   .\scripts\deploy.ps1                    # start both services
#   .\scripts\deploy.ps1 -Teardown           # stop background processes
#   .\scripts\deploy.ps1 -Source "C:\path" -DaemonPort 19281
#
# Requires: PowerShell 5+, Rust (cargo), Node.js (npm)

param(
    [string]$Source = "",
    [int]$DaemonPort = 19280,
    [string]$DaemonBind = "127.0.0.1",
    [string]$StoreDir = "$env:TEMP\chatcodex-daemon",
    [string]$WorkspaceRoot = "",
    [int]$GatewayPort = 3000,
    [string]$GatewayHost = "127.0.0.1",
    [ValidateSet("stdio", "http")]
    [string]$Transport = "http",
    [switch]$Hybrid,
    [string]$HybridUrl = "",
    [string]$HybridModel = "",
    [switch]$Teardown,
    [switch]$Help
)

$ErrorActionPreference = "Stop"

if ($Help) {
    @"
ChatCodex Deployment Script (Windows)

Usage: .\scripts\deploy.ps1 [options]

Options:
  -Source DIR         Path to the ChatCodex repo (default: derived from script location)
  -DaemonPort PORT    Port for the daemon (default: 19280)
  -DaemonBind HOST     Bind address for the daemon (default: 127.0.0.1)
  -StoreDir DIR       Directory for daemon SQLite store (default: $env:TEMP\chatcodex-daemon)
  -WorkspaceRoot DIR  Workspace root directory (default: none)
  -GatewayPort PORT   Port for the MCP HTTP gateway (default: 3000)
  -GatewayHost HOST   Host for the MCP gateway (default: 127.0.0.1)
  -Transport MODE     Transport: 'stdio' or 'http' (default: http)
  -Hybrid             Enable hybrid mode (requires -HybridUrl and -HybridModel)
  -HybridUrl URL      Worker LLM base URL
  -HybridModel MODEL  Worker model name
  -Teardown          Stop background processes and exit
  -Help              Show this help

Environment variables also work (override CLI args):
  CHATCODEX_DAEMON_PORT, CHATCODEX_WORKSPACE_ROOT, CHATCODEX_HYBRID_ENABLED,
  CHATCODEX_HYBRID_PROVIDER_URL, CHATCODEX_HYBRID_MODEL

Examples:
  # Minimal local setup
  .\scripts\deploy.ps1

  # Remote accessible (browser ChatGPT)
  .\scripts\deploy.ps1 -GatewayHost "0.0.0.0" -GatewayPort 3000

  # With hybrid mode (Ollama)
  .\scripts\deploy.ps1 -Hybrid -HybridUrl "http://localhost:11434/v1" -HybridModel "qwen2.5-coder"

  # Teardown
  .\scripts\deploy.ps1 -Teardown
"@
    exit 0
}

$script:DaemonJob = $null
$script:GatewayJob = $null

function Write-Log {
    param([string]$Message)
    Write-Host "[chatcodex-deploy] $Message" -ForegroundColor Cyan
}

function Write-Warn {
    param([string]$Message)
    Write-Host "[chatcodex-deploy] WARNING: $Message" -ForegroundColor Yellow
}

function Write-Err {
    param([string]$Message)
    Write-Host "[chatcodex-deploy] ERROR: $Message" -ForegroundColor Red
    Do-Teardown
    exit 1
}

# Resolve source directory
if (-not $Source) {
    $Source = Split-Path -Parent $MyInvocation.MyCommand.Path
    $Source = Split-Path -Parent $Source
}
$Source = (Resolve-Path $Source -ErrorAction SilentlyContinue).Path
if (-not $Source) {
    Write-Err "Source directory not found: $Source"
}
$CargoToml = Join-Path $Source "Cargo.toml"
if (-not (Test-Path $CargoToml)) {
    Write-Err "Not a ChatCodex repo (no Cargo.toml): $Source"
}

# Env var overrides
if ($env:CHATCODEX_DAEMON_PORT) { $DaemonPort = [int]$env:CHATCODEX_DAEMON_PORT }
if ($env:CHATCODEX_WORKSPACE_ROOT) { $WorkspaceRoot = $env:CHATCODEX_WORKSPACE_ROOT }
if ($env:CHATCODEX_HYBRID_ENABLED) { $Hybrid = $true }
if ($env:CHATCODEX_HYBRID_PROVIDER_URL) { $HybridUrl = $env:CHATCODEX_HYBRID_PROVIDER_URL }
if ($env:CHATCODEX_HYBRID_MODEL) { $HybridModel = $env:CHATCODEX_HYBRID_MODEL }

$DaemonBindAddr = "${DaemonBind}:${DaemonPort}"
$DaemonBin = Join-Path $Source "target\release\deterministic-daemon.exe"
if (-not (Test-Path $DaemonBin)) {
    $DaemonBin = Join-Path $Source "target\debug\deterministic-daemon.exe"
}
$GatewayDir = Join-Path $Source "apps\chatgpt-mcp"
$GatewayBin = Join-Path $GatewayDir "dist\index.js"

$DaemonLog = "$env:TEMP\chatcodex-daemon.log"
$GatewayLog = "$env:TEMP\chatcodex-mcp.log"

# Create directories
New-Item -ItemType Directory -Force -Path $StoreDir | Out-Null

# -----------------------------------------------------------------------
# Teardown
# -----------------------------------------------------------------------

function Do-Teardown {
    Write-Log "teardown requested"
    if ($script:DaemonJob) {
        Write-Log "stopping daemon..."
        Stop-Job -Job $script:DaemonJob -ErrorAction SilentlyContinue
        Remove-Job -Job $script:DaemonJob -Force -ErrorAction SilentlyContinue
    }
    if ($script:GatewayJob) {
        Write-Log "stopping MCP gateway..."
        Stop-Job -Job $script:GatewayJob -ErrorAction SilentlyContinue
        Remove-Job -Job $script:GatewayJob -Force -ErrorAction SilentlyContinue
    }
    Write-Log "teardown complete"
}

if ($Teardown) {
    Do-Teardown
    exit 0
}

# -----------------------------------------------------------------------
# Build daemon
# -----------------------------------------------------------------------

Write-Log "building daemon..."
$build = Start-Process -FilePath "cargo" `
    -ArgumentList "build", "--release", "-p", "deterministic-daemon", "--manifest-path", $Source\Cargo.toml `
    -WorkingDirectory $Source `
    -NoNewWindow -PassThru -Wait
if ($build.ExitCode -ne 0) {
    Write-Err "daemon build failed (exit code $($build.ExitCode))"
}
if (-not (Test-Path $DaemonBin)) {
    Write-Err "daemon binary not found after build: $DaemonBin"
}
Write-Log "daemon built"

# -----------------------------------------------------------------------
# Build gateway
# -----------------------------------------------------------------------

if ($Transport -eq "http") {
    Write-Log "building MCP gateway..."
    Push-Location $GatewayDir
    try {
        npm install --silent
        npm run build
        if (-not (Test-Path $GatewayBin)) {
            Write-Err "MCP gateway build failed (no dist/index.js)"
        }
    }
    finally {
        Pop-Location
    }
    Write-Log "gateway built"
}

# -----------------------------------------------------------------------
# Build environment blocks
# -----------------------------------------------------------------------

$daemonEnv = @{
    "DETERMINISTIC_BIND" = $DaemonBindAddr
    "DETERMINISTIC_STORE_DIR" = $StoreDir
}
if ($WorkspaceRoot) {
    $daemonEnv["DETERMINISTIC_WORKSPACE_ROOT"] = $WorkspaceRoot
}
if ($Hybrid) {
    $daemonEnv["CHATCODEX_HYBRID_ENABLED"] = "true"
    if ($HybridUrl) { $daemonEnv["CHATCODEX_HYBRID_PROVIDER_URL"] = $HybridUrl }
    if ($HybridModel) { $daemonEnv["CHATCODEX_HYBRID_MODEL"] = $HybridModel }
}

$gatewayEnv = @{
    "NODE_ENV" = "production"
    "DETERMINISTIC_DAEMON_URL" = "http://127.0.0.1:${DaemonPort}"
    "MCP_TRANSPORT" = $Transport
    "PORT" = $GatewayPort
    "HOST" = $GatewayHost
}

# -----------------------------------------------------------------------
# Start daemon
# -----------------------------------------------------------------------

Write-Log "starting daemon on $DaemonBindAddr..."

$daemonEnvScript = ($daemonEnv.GetEnumerator() | ForEach-Object { "$($_.Key)=$($_.Value)" }) -join "; "
$daemonCmd = [System.Diagnostics.ProcessStartInfo]::new()
$daemonCmd.FileName = $DaemonBin
$daemonCmd.WorkingDirectory = $Source
$daemonCmd.UseShellExecute = $false
$daemonCmd.RedirectStandardOutput = $true
$daemonCmd.RedirectStandardError = $true
$daemonCmd.CreateNoWindow = $true
foreach ($k in $daemonEnv.Keys) {
    $daemonCmd.EnvironmentVariables[$k] = $daemonEnv[$k]
}

$daemonProc = [System.Diagnostics.Process]::Start($daemonCmd)
$script:DaemonJob = $daemonProc

Start-Sleep 2
if ($daemonProc.HasExited) {
    $stdout = $daemonProc.StandardOutput.ReadToEnd()
    $stderr = $daemonProc.StandardError.ReadToEnd()
    Write-Err "daemon exited immediately (exit code $($daemonProc.ExitCode))`nstdout: $stdout`nstderr: $stderr"
}
Write-Log "daemon started (PID $($daemonProc.Id))"

# -----------------------------------------------------------------------
# Start gateway
# -----------------------------------------------------------------------

if ($Transport -eq "http") {
    Write-Log "starting MCP gateway on ${GatewayHost}:${GatewayPort}..."

    $gatewayCmd = [System.Diagnostics.ProcessStartInfo]::new()
    $gatewayCmd.FileName = "node"
    $gatewayCmd.Arguments = $GatewayBin
    $gatewayCmd.WorkingDirectory = $GatewayDir
    $gatewayCmd.UseShellExecute = $false
    $gatewayCmd.RedirectStandardOutput = $true
    $gatewayCmd.RedirectStandardError = $true
    $gatewayCmd.CreateNoWindow = $true
    foreach ($k in $gatewayEnv.Keys) {
        $gatewayCmd.EnvironmentVariables[$k] = $gatewayEnv[$k]
    }

    $gatewayProc = [System.Diagnostics.Process]::Start($gatewayCmd)
    $script:GatewayJob = $gatewayProc

    Start-Sleep 2
    if (-not $gatewayProc.HasExited) {
        Write-Log "MCP gateway started (PID $($gatewayProc.Id))"
    }
    else {
        Write-Warn "gateway exited immediately (exit code $($gatewayProc.ExitCode))"
    }

    $gatewayUrl = "http://${GatewayHost}:${GatewayPort}/mcp"
    $healthzUrl = "http://${GatewayHost}:${GatewayPort}/healthz"
}
else {
    Write-Log "starting MCP gateway in stdio mode..."

    $gatewayCmd = [System.Diagnostics.ProcessStartInfo]::new()
    $gatewayCmd.FileName = "node"
    $gatewayCmd.Arguments = $GatewayBin
    $gatewayCmd.WorkingDirectory = $GatewayDir
    $gatewayCmd.UseShellExecute = $false
    $gatewayCmd.RedirectStandardOutput = $true
    $gatewayCmd.RedirectStandardError = $true
    $gatewayCmd.CreateNoWindow = $true
    foreach ($k in $gatewayEnv.Keys) {
        $gatewayCmd.EnvironmentVariables[$k] = $gatewayEnv[$k]
    }

    $gatewayProc = [System.Diagnostics.Process]::Start($gatewayCmd)
    $script:GatewayJob = $gatewayProc

    Start-Sleep 2
    Write-Log "MCP gateway started in stdio mode (PID $($gatewayProc.Id))"
    $gatewayUrl = "stdio (process PID $($gatewayProc.Id))"
    $healthzUrl = ""
}

# -----------------------------------------------------------------------
# Print summary
# -----------------------------------------------------------------------

Write-Host ""
Write-Host "============================================" -ForegroundColor Green
Write-Host "  ChatCodex stack is running" -ForegroundColor Green
Write-Host "============================================"
Write-Host ""
Write-Host "  Daemon PID:      $($daemonProc.Id)"
Write-Host "  Daemon URL:     http://127.0.0.1:${DaemonPort}"
Write-Host "  Store dir:      $StoreDir"
Write-Host ""
if ($gatewayProc -and -not $gatewayProc.HasExited) {
    Write-Host "  Gateway PID:     $($gatewayProc.Id)"
    Write-Host "  Gateway URL:    $gatewayUrl"
    Write-Host ""
}
if ($Hybrid) {
    Write-Host "  Hybrid mode:     ENABLED (url: $HybridUrl, model: $HybridModel)"
    Write-Host ""
}
Write-Host "--------------------------------------------"
Write-Host "  To verify:"
Write-Host "    curl http://127.0.0.1:${DaemonPort}/healthz"
if ($healthzUrl) {
    Write-Host "    curl $healthzUrl"
}
Write-Host ""
Write-Host "  To register in ChatGPT Desktop:"
if ($Transport -eq "http") {
    Write-Host "    MCP URL: $gatewayUrl"
}
else {
    Write-Host "    Run in $GatewayDir: `$env:MCP_TRANSPORT='stdio'; node dist/index.js"
    Write-Host "    (ChatGPT Desktop auto-discovers stdio servers)"
}
Write-Host ""
Write-Host "  To teardown:"
Write-Host "    .\scripts\deploy.ps1 -Teardown"
Write-Host "============================================"
Write-Host ""

# Wait for either process to exit
try {
    $daemonProc.WaitForExit()
}
catch {}
Write-Log "a background process exited, shutting down..."
Do-Teardown