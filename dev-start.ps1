# dev-start.ps1 — поднимает локальный стек Parcer на этой машине (без Docker).
# Сервисы: PostgreSQL 15, Redis 8, FastAPI (uvicorn), Vite dev server.
# Всё user-scope через scoop, админ не требуется.
#
#   .\dev-start.ps1          — запустить всё
#   .\dev-start.ps1 -Stop    — остановить backend/frontend/redis (postgres оставить)
#   .\dev-start.ps1 -Status  — показать состояние

param(
    [switch]$Stop,
    [switch]$Status
)

$ErrorActionPreference = 'Stop'

$Root     = $PSScriptRoot
$PgBin    = 'C:\Dev\scoop\apps\postgresql15\current\bin'
$PgData   = 'C:\Dev\scoop\apps\postgresql15\current\data'
$RedisExe = 'C:\Dev\scoop\apps\redis\current\redis-server.exe'
$RedisCli = 'C:\Dev\scoop\apps\redis\current\redis-cli.exe'
$RedisDir = 'C:\Dev\redis-data'
$VenvPy   = Join-Path $Root 'backend\.venv\Scripts\python.exe'

function Test-Port([int]$Port) {
    # Vite binds to ::1 only, postgres/redis to 127.0.0.1 — probe both.
    foreach ($h in @('127.0.0.1', '::1')) {
        try {
            $client = [System.Net.Sockets.TcpClient]::new()
            $ok = $client.ConnectAsync($h, $Port).Wait(700)
            $client.Close()
            if ($ok) { return $true }
        } catch { }
    }
    return $false
}

function Show-Status {
    "PostgreSQL :5432  " + $(if (Test-Port 5432) { 'UP' } else { 'down' })
    "Redis      :6379  " + $(if (Test-Port 6379) { 'UP' } else { 'down' })
    "Backend    :8000  " + $(if (Test-Port 8000) { 'UP' } else { 'down' })
    "Frontend   :5173  " + $(if (Test-Port 5173) { 'UP' } else { 'down' })
}

if ($Status) { Show-Status; return }

if ($Stop) {
    Get-CimInstance Win32_Process -Filter "Name='python.exe' OR Name='node.exe'" |
        Where-Object { $_.CommandLine -match 'uvicorn|vite' } |
        ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
    Get-Process redis-server -ErrorAction SilentlyContinue | Stop-Process -Force
    'Stopped backend / frontend / redis (postgres left running).'
    return
}

# --- PostgreSQL ---
if (-not (Test-Port 5432)) {
    & "$PgBin\pg_ctl.exe" -D $PgData -l "$PgData\server.log" -w start | Out-Null
    'postgres: started'
} else { 'postgres: already running' }

# --- Redis ---
if (-not (Test-Port 6379)) {
    New-Item -ItemType Directory -Force -Path $RedisDir | Out-Null
    Start-Process -FilePath $RedisExe `
        -ArgumentList '--port','6379','--requirepass','localdev','--dir',$RedisDir,'--save','900','1' `
        -WindowStyle Hidden
    'redis: started'
} else { 'redis: already running' }

# --- Backend (FastAPI) ---
if (-not (Test-Port 8000)) {
    $env:PYTHONPATH = Join-Path $Root 'backend'
    Start-Process -FilePath $VenvPy `
        -ArgumentList '-m','uvicorn','api.main:app','--host','127.0.0.1','--port','8000','--reload' `
        -WorkingDirectory (Join-Path $Root 'backend') `
        -RedirectStandardOutput (Join-Path $Root 'backend\uvicorn.out.log') `
        -RedirectStandardError  (Join-Path $Root 'backend\uvicorn.err.log') `
        -WindowStyle Hidden
    'backend: starting on http://127.0.0.1:8000 (logs: backend\uvicorn.*.log)'
} else { 'backend: already running' }

# --- Frontend (Vite) ---
if (-not (Test-Port 5173)) {
    Start-Process -FilePath 'C:\Dev\scoop\apps\nodejs-lts\current\npm.cmd' `
        -ArgumentList 'run','dev' `
        -WorkingDirectory (Join-Path $Root 'frontend') `
        -RedirectStandardOutput (Join-Path $Root 'frontend\vite.out.log') `
        -RedirectStandardError  (Join-Path $Root 'frontend\vite.err.log') `
        -WindowStyle Hidden
    'frontend: starting on http://localhost:5173 (logs: frontend\vite.*.log)'
} else { 'frontend: already running' }

Start-Sleep -Seconds 12
''
Show-Status
''
'UI:    http://localhost:5173     login: admin / Adm1n#Local2026'
'Docs:  http://127.0.0.1:8000/docs'
