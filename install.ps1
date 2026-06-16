# Instalador do DA (Deploy Automático) para Windows.
#
#   irm https://raw.githubusercontent.com/KingTimer12/da-cli/main/install.ps1 | iex
#
# Variáveis opcionais (defina antes de rodar):
#   $env:DA_VERSION      tag específica (ex: v0.1.0). Default: latest.
#   $env:DA_INSTALL_DIR  diretório de instalação. Default: %LOCALAPPDATA%\Programs\da.

$ErrorActionPreference = 'Stop'

$repo = 'KingTimer12/da-cli'
$bin  = 'da'

if (-not [Environment]::Is64BitOperatingSystem) {
    throw 'apenas Windows x86_64 é suportado'
}
$target = 'x86_64-pc-windows-msvc'

$version = if ($env:DA_VERSION) { $env:DA_VERSION } else { 'latest' }
$url = if ($version -eq 'latest') {
    "https://github.com/$repo/releases/latest/download/$bin-$target.zip"
} else {
    "https://github.com/$repo/releases/download/$version/$bin-$target.zip"
}

$tmp = Join-Path $env:TEMP ("da-" + [guid]::NewGuid().ToString())
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
try {
    $zip = Join-Path $tmp 'da.zip'
    Write-Host "baixando $url"
    Invoke-WebRequest -Uri $url -OutFile $zip
    Expand-Archive -Path $zip -DestinationPath $tmp -Force

    $dir = if ($env:DA_INSTALL_DIR) { $env:DA_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'Programs\da' }
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    Copy-Item (Join-Path $tmp "$bin-$target\$bin.exe") (Join-Path $dir 'da.exe') -Force

    Write-Host "instalado em $dir\da.exe"

    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (($userPath -split ';') -notcontains $dir) {
        [Environment]::SetEnvironmentVariable('Path', "$userPath;$dir", 'User')
        Write-Host "adicionado ao PATH do usuário — reabra o terminal para usar 'da'"
    }
    Write-Host "rode: da --help"
}
finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
