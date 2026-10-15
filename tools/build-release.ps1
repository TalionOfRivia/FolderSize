# Builds the release exe without the builder's local paths inside it.
# The compiler records source file paths (for error messages); this rewrites
# them so your Windows user name and folder layout don't end up in the exe.
#
#   powershell -File tools\build-release.ps1
# Output: target\release\foldersize.exe

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE '.cargo' }
$rustup = if ($env:RUSTUP_HOME) { $env:RUSTUP_HOME } else { Join-Path $env:USERPROFILE '.rustup' }

$env:RUSTFLAGS = @(
    "--remap-path-prefix=$repo=foldersize"
    "--remap-path-prefix=$cargoHome=cargo"
    "--remap-path-prefix=$rustup=rustup"
    "--remap-path-prefix=$env:USERPROFILE=user"
) -join ' '

Push-Location $repo
try {
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
    $targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $repo 'target' }
    $exe = Join-Path $targetDir 'release\foldersize.exe'
    $text = [System.Text.Encoding]::ASCII.GetString([System.IO.File]::ReadAllBytes($exe))
    foreach ($p in @($env:USERNAME, $env:USERPROFILE, $repo) | Where-Object { $_ }) {
        if ($text.IndexOf($p, [StringComparison]::OrdinalIgnoreCase) -ge 0) {
            Write-Warning "The exe still contains '$p'"
        }
    }
    Write-Host "Built $exe"
} finally {
    Pop-Location
}
